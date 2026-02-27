use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;

use super::audio::AudioOutput;
use super::config::Config;

const GG_WIDTH: usize = 160;
const GG_HEIGHT: usize = 144;
const GG_X_OFFSET: usize = 48;
const GG_Y_OFFSET: usize = 24;
const CYCLES_PER_SCANLINE: u32 = 228;
const SCANLINES_PER_FRAME: u16 = 262;
const FRAME_DURATION: Duration = Duration::from_micros(16_742); // ~59.73 fps (NTSC GG)

pub struct EmulatorState {
    pub cpu: Cpu,
    pub bus: Bus,
    pub rom_path: PathBuf,
}

pub struct RustyGearApp {
    pub emu: Option<EmulatorState>,
    pub config: Config,
    pub paused: bool,
    pub frame_step: bool,
    pub speed: Speed,
    pub audio: Option<AudioOutput>,
    pub game_texture: Option<egui::TextureHandle>,
    pub last_frame_time: Instant,
    pub icon_texture: Option<egui::TextureHandle>,

    // Debug panel visibility
    pub show_cpu_debug: bool,
    pub show_vram_viewer: bool,
    pub show_tile_viewer: bool,
    pub show_about: bool,
    pub show_input_settings: bool,

    // Input binding state
    pub awaiting_key_for: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Speed {
    Normal,
    Fast,
    Uncapped,
}

impl RustyGearApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config = Config::load();
        let audio = AudioOutput::new();
        if audio.is_none() {
            eprintln!("Warning: failed to initialize audio output");
        }

        Self {
            emu: None,
            config,
            paused: false,
            frame_step: false,
            speed: Speed::Normal,
            audio,
            game_texture: None,
            last_frame_time: Instant::now(),
            icon_texture: None,
            show_cpu_debug: false,
            show_vram_viewer: false,
            show_tile_viewer: false,
            show_about: false,
            show_input_settings: false,
            awaiting_key_for: None,
        }
    }

    pub fn load_rom(&mut self, rom_path: PathBuf) {
        let rom = match std::fs::read(&rom_path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Failed to read ROM: {}", e);
                return;
            }
        };

        let bios = self.config.bios_path.as_ref().and_then(|p| {
            std::fs::read(p).ok()
        });

        let cartridge = Cartridge::new(rom);
        let mut bus = Bus::new(cartridge, bios);
        let cpu = Cpu::new();

        if let Some(ref audio) = self.audio {
            bus.psg.set_sample_rate(audio.sample_rate);
        }

        self.emu = Some(EmulatorState {
            cpu,
            bus,
            rom_path: rom_path.clone(),
        });
        self.paused = false;
        self.config.add_recent_rom(rom_path);
    }

    pub fn reset(&mut self) {
        if let Some(ref emu) = self.emu {
            let rom_path = emu.rom_path.clone();
            self.load_rom(rom_path);
        }
    }

    pub fn run_frame(&mut self) {
        let emu = match self.emu.as_mut() {
            Some(e) => e,
            None => return,
        };

        for _ in 0..SCANLINES_PER_FRAME {
            let mut cycles_this_line: u32 = 0;
            while cycles_this_line < CYCLES_PER_SCANLINE {
                let c = emu.cpu.step(&mut emu.bus);
                if c == 0 { break; }
                cycles_this_line += c;
            }
            emu.bus.vdp.run_scanline();
            emu.bus.irq_pending = emu.bus.vdp.irq_pending();
            emu.cpu.handle_interrupts(&mut emu.bus);
        }

        // Generate audio samples at the device's native sample rate
        if let Some(ref audio) = self.audio {
            let samples_per_frame = audio.sample_rate as usize / 60;
            let mut samples = Vec::with_capacity(samples_per_frame);
            let volume = if self.config.muted { 0.0 } else { self.config.volume };
            for _ in 0..samples_per_frame {
                let (l, r) = emu.bus.psg.generate_sample();
                let l = (l as f32 * volume) as i16;
                let r = (r as f32 * volume) as i16;
                samples.push((l, r));
            }
            audio.push_samples(&samples);
        }
    }

    pub fn update_game_texture(&mut self, ctx: &egui::Context) {
        let emu = match self.emu.as_ref() {
            Some(e) => e,
            None => return,
        };

        let mut pixels = vec![egui::Color32::BLACK; GG_WIDTH * GG_HEIGHT];
        for y in 0..GG_HEIGHT {
            for x in 0..GG_WIDTH {
                let src_x = x + GG_X_OFFSET;
                let src_y = y + GG_Y_OFFSET;
                let pixel = emu.bus.vdp.framebuffer[src_y * 256 + src_x];
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;
                pixels[y * GG_WIDTH + x] = egui::Color32::from_rgb(r, g, b);
            }
        }

        let image = egui::ColorImage {
            size: [GG_WIDTH, GG_HEIGHT],
            pixels,
        };

        let texture_options = egui::TextureOptions::NEAREST;

        match self.game_texture.as_mut() {
            Some(tex) => tex.set(image, texture_options),
            None => {
                self.game_texture =
                    Some(ctx.load_texture("game_screen", image, texture_options));
            }
        }
    }
}

impl eframe::App for RustyGearApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Run emulation with proper frame timing
        if self.emu.is_some() && (!self.paused || self.frame_step) {
            let now = Instant::now();

            let frame_duration = match self.speed {
                Speed::Normal => FRAME_DURATION,
                Speed::Fast => FRAME_DURATION / 2,
                Speed::Uncapped => Duration::ZERO,
            };

            if self.frame_step {
                self.run_frame();
                self.frame_step = false;
                self.last_frame_time = now;
                self.update_game_texture(ctx);
            } else {
                // Run as many frames as needed to catch up, capped at 3
                // to avoid spiral-of-death when the system is too slow.
                let mut frames_run = 0;
                while self.last_frame_time + frame_duration <= now && frames_run < 3 {
                    self.run_frame();
                    self.last_frame_time += frame_duration;
                    frames_run += 1;
                }

                // If we fell too far behind (e.g. window was minimized), reset
                if self.last_frame_time + frame_duration * 3 < now {
                    self.last_frame_time = now;
                }

                if frames_run > 0 {
                    self.update_game_texture(ctx);
                }
            }

            // Schedule next repaint at the right time
            if !self.paused {
                let next = (self.last_frame_time + frame_duration)
                    .saturating_duration_since(now);
                ctx.request_repaint_after(next);
            }
        }

        self.draw_menu_bar(ctx);

        // Handle keyboard input
        self.handle_input(ctx);

        // Draw input settings window if open
        self.draw_input_settings(ctx);

        // Draw about dialog if open
        if self.show_about {
            // Lazily load the icon texture for the about dialog
            if self.icon_texture.is_none() {
                let png_bytes = include_bytes!("../../resources/icon.png");
                if let Ok(img) = image::load_from_memory(png_bytes) {
                    let rgba = img.into_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = rgba.pixels().map(|p| {
                        egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
                    }).collect();
                    let image = egui::ColorImage { size, pixels };
                    self.icon_texture = Some(ctx.load_texture(
                        "app_icon",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }

            egui::Window::new("About RustyGear")
                .open(&mut self.show_about)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        if let Some(ref tex) = self.icon_texture {
                            ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(64.0, 64.0)));
                            ui.add_space(8.0);
                        }
                        ui.heading("RustyGear");
                        ui.label("v0.1.0");
                        ui.add_space(4.0);
                        ui.label("A Sega Game Gear emulator written in Rust");
                    });
                });
        }

        // Debug panels
        self.draw_cpu_debug(ctx);
        self.draw_vram_viewer(ctx);
        self.draw_tile_viewer(ctx);

        // Central panel with game display
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref texture) = self.game_texture {
                let scale = self.config.scale as f32;
                let size = egui::vec2(GG_WIDTH as f32 * scale, GG_HEIGHT as f32 * scale);
                let available = ui.available_size();
                let offset = egui::vec2(
                    (available.x - size.x).max(0.0) / 2.0,
                    (available.y - size.y).max(0.0) / 2.0,
                );
                ui.add_space(offset.y);
                ui.horizontal(|ui| {
                    ui.add_space(offset.x);
                    ui.image(egui::load::SizedTexture::new(texture.id(), size));
                });

                if self.paused {
                    let rect = ui.clip_rect();
                    let center = rect.center();
                    ui.painter().text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        "PAUSED",
                        egui::FontId::proportional(32.0),
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                    );
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No ROM loaded. Use File > Open ROM to get started.");
                });
            }
        });
    }
}
