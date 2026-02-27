use eframe::egui;
use std::path::PathBuf;

use super::app::{RustyGearApp, Speed};

impl RustyGearApp {
    pub fn draw_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                self.file_menu(ui);
                self.emulation_menu(ui);
                self.options_menu(ui, ctx);
                self.debug_menu(ui);
                self.help_menu(ui);
            });
        });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("Open ROM...").clicked() {
                ui.close_menu();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Game Gear ROM", &["gg"])
                    .pick_file()
                {
                    self.load_rom(path);
                }
            }

            ui.menu_button("Open Recent", |ui| {
                if self.config.recent_roms.is_empty() {
                    ui.label("(no recent ROMs)");
                } else {
                    let roms: Vec<PathBuf> = self.config.recent_roms.clone();
                    for rom in &roms {
                        let name = rom
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| rom.display().to_string());
                        if ui.button(&name).clicked() {
                            ui.close_menu();
                            self.load_rom(rom.clone());
                        }
                    }
                }
            });

            if ui.button("Load BIOS...").clicked() {
                ui.close_menu();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("BIOS", &["gg", "bin"])
                    .pick_file()
                {
                    self.config.bios_path = Some(path);
                    self.config.save();
                }
            }

            ui.separator();

            if ui.button("Exit").clicked() {
                std::process::exit(0);
            }
        });
    }

    fn emulation_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Emulation", |ui| {
            let has_rom = self.emu.is_some();

            let pause_text = if self.paused { "Resume" } else { "Pause" };
            if ui.add_enabled(has_rom, egui::Button::new(format!("{} (P)", pause_text))).clicked() {
                self.paused = !self.paused;
                ui.close_menu();
            }

            if ui.add_enabled(has_rom, egui::Button::new("Reset")).clicked() {
                self.reset();
                ui.close_menu();
            }

            if ui.add_enabled(has_rom && self.paused, egui::Button::new("Frame Step (N)")).clicked() {
                self.frame_step = true;
                ui.close_menu();
            }

            ui.separator();

            ui.menu_button("Speed", |ui| {
                if ui.radio_value(&mut self.speed, Speed::Normal, "Normal").clicked() {
                    ui.close_menu();
                }
                if ui.radio_value(&mut self.speed, Speed::Fast, "Fast (2x)").clicked() {
                    ui.close_menu();
                }
                if ui.radio_value(&mut self.speed, Speed::Uncapped, "Uncapped").clicked() {
                    ui.close_menu();
                }
            });
        });
    }

    fn options_menu(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.menu_button("Options", |ui| {
            ui.menu_button("Video Scale", |ui| {
                for s in 1..=6u32 {
                    let label = format!("{}x", s);
                    if ui.radio_value(&mut self.config.scale, s, &label).clicked() {
                        self.config.save();
                        ui.close_menu();
                    }
                }
            });

            if ui.checkbox(&mut self.config.muted, "Mute Audio (M)").changed() {
                self.config.save();
            }

            ui.horizontal(|ui| {
                ui.label("Volume:");
                if ui.add(egui::Slider::new(&mut self.config.volume, 0.0..=1.0)).changed() {
                    self.config.save();
                }
            });

            ui.separator();

            if ui.button("Input Bindings...").clicked() {
                self.show_input_settings = true;
                ui.close_menu();
            }
        });
    }

    fn debug_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Debug", |ui| {
            if ui.checkbox(&mut self.show_cpu_debug, "CPU State").clicked() {
                ui.close_menu();
            }
            if ui.checkbox(&mut self.show_vram_viewer, "VRAM Viewer").clicked() {
                ui.close_menu();
            }
            if ui.checkbox(&mut self.show_tile_viewer, "Tile Viewer").clicked() {
                ui.close_menu();
            }
        });
    }

    fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui.button("About").clicked() {
                self.show_about = true;
                ui.close_menu();
            }
        });
    }
}
