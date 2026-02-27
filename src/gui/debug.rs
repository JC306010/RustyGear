use eframe::egui;

use crate::cpu::{FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_Z};

use super::app::RustyGearApp;

impl RustyGearApp {
    pub fn draw_cpu_debug(&mut self, ctx: &egui::Context) {
        if !self.show_cpu_debug {
            return;
        }

        let mut open = self.show_cpu_debug;
        egui::Window::new("CPU State")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let emu = match self.emu.as_ref() {
                    Some(e) => e,
                    None => {
                        ui.label("No ROM loaded");
                        return;
                    }
                };

                let cpu = &emu.cpu;

                ui.style_mut().override_font_id =
                    Some(egui::FontId::monospace(14.0));

                egui::Grid::new("regs_main").num_columns(4).show(ui, |ui| {
                    ui.label("AF:");
                    ui.label(format!("{:04X}", cpu.af()));
                    ui.label("AF':");
                    ui.label(format!(
                        "{:04X}",
                        ((cpu.a_prime as u16) << 8) | cpu.f_prime as u16
                    ));
                    ui.end_row();

                    ui.label("BC:");
                    ui.label(format!("{:04X}", cpu.bc()));
                    ui.label("BC':");
                    ui.label(format!(
                        "{:04X}",
                        ((cpu.b_prime as u16) << 8) | cpu.c_prime as u16
                    ));
                    ui.end_row();

                    ui.label("DE:");
                    ui.label(format!("{:04X}", cpu.de()));
                    ui.label("DE':");
                    ui.label(format!(
                        "{:04X}",
                        ((cpu.d_prime as u16) << 8) | cpu.e_prime as u16
                    ));
                    ui.end_row();

                    ui.label("HL:");
                    ui.label(format!("{:04X}", cpu.hl()));
                    ui.label("HL':");
                    ui.label(format!(
                        "{:04X}",
                        ((cpu.h_prime as u16) << 8) | cpu.l_prime as u16
                    ));
                    ui.end_row();
                });

                ui.separator();

                egui::Grid::new("regs_special").num_columns(4).show(ui, |ui| {
                    ui.label("IX:");
                    ui.label(format!("{:04X}", cpu.ix));
                    ui.label("IY:");
                    ui.label(format!("{:04X}", cpu.iy));
                    ui.end_row();

                    ui.label("SP:");
                    ui.label(format!("{:04X}", cpu.sp));
                    ui.label("PC:");
                    ui.label(format!("{:04X}", cpu.pc));
                    ui.end_row();

                    ui.label("I:");
                    ui.label(format!("  {:02X}", cpu.i));
                    ui.label("R:");
                    ui.label(format!("  {:02X}", cpu.r));
                    ui.end_row();
                });

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Flags:");
                    let flag_names = [
                        ("S", cpu.flag(FLAG_S)),
                        ("Z", cpu.flag(FLAG_Z)),
                        ("H", cpu.flag(FLAG_H)),
                        ("P", cpu.flag(FLAG_PV)),
                        ("N", cpu.flag(FLAG_N)),
                        ("C", cpu.flag(FLAG_C)),
                    ];
                    for (name, set) in &flag_names {
                        let text = if *set {
                            egui::RichText::new(*name).strong().color(egui::Color32::GREEN)
                        } else {
                            egui::RichText::new(*name).color(egui::Color32::GRAY)
                        };
                        ui.label(text);
                    }
                });

                ui.separator();

                egui::Grid::new("regs_state").num_columns(4).show(ui, |ui| {
                    ui.label("IFF1:");
                    ui.label(if cpu.iff1 { "on" } else { "off" });
                    ui.label("IFF2:");
                    ui.label(if cpu.iff2 { "on" } else { "off" });
                    ui.end_row();

                    ui.label("IM:");
                    ui.label(format!("{}", cpu.im));
                    ui.label("Halted:");
                    ui.label(if cpu.halted { "yes" } else { "no" });
                    ui.end_row();
                });
            });
        self.show_cpu_debug = open;
    }

    pub fn draw_vram_viewer(&mut self, ctx: &egui::Context) {
        if !self.show_vram_viewer {
            return;
        }

        let mut open = self.show_vram_viewer;
        egui::Window::new("VRAM Viewer")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 460.0])
            .show(ctx, |ui| {
                let emu = match self.emu.as_ref() {
                    Some(e) => e,
                    None => {
                        ui.label("No ROM loaded");
                        return;
                    }
                };

                let vdp = &emu.bus.vdp;

                // Name table base address
                let nt_base = ((vdp.registers[2] as u16 & 0x0E) << 10) & 0x3800;

                ui.label(format!(
                    "Name Table Base: ${:04X}  Scroll: H={} V={}",
                    nt_base, vdp.registers[8], vdp.registers[9]
                ));

                // Render name table as a 32x28 tile grid (256x224 pixels)
                let nt_width = 32usize;
                let nt_height = 28usize;
                let tile_size = 8usize;
                let img_w = nt_width * tile_size;
                let img_h = nt_height * tile_size;

                let mut pixels = vec![egui::Color32::BLACK; img_w * img_h];

                for ty in 0..nt_height {
                    for tx in 0..nt_width {
                        let nt_addr = nt_base + ((ty * nt_width + tx) * 2) as u16;
                        let word_lo = vdp.vram[nt_addr as usize] as u16;
                        let word_hi = vdp.vram[(nt_addr + 1) as usize] as u16;
                        let word = word_lo | (word_hi << 8);

                        let tile_idx = word & 0x01FF;
                        let palette = if word & 0x0800 != 0 { 16 } else { 0 };
                        let h_flip = word & 0x0200 != 0;
                        let v_flip = word & 0x0400 != 0;

                        let tile_addr = (tile_idx * 32) as usize;

                        for row in 0..8usize {
                            let src_row = if v_flip { 7 - row } else { row };
                            let byte_offset = tile_addr + src_row * 4;
                            if byte_offset + 3 >= vdp.vram.len() {
                                continue;
                            }

                            let b0 = vdp.vram[byte_offset];
                            let b1 = vdp.vram[byte_offset + 1];
                            let b2 = vdp.vram[byte_offset + 2];
                            let b3 = vdp.vram[byte_offset + 3];

                            for col in 0..8usize {
                                let src_col = if h_flip { col } else { 7 - col };
                                let bit = 1 << src_col;
                                let color_idx = ((b0 & bit != 0) as usize)
                                    | (((b1 & bit != 0) as usize) << 1)
                                    | (((b2 & bit != 0) as usize) << 2)
                                    | (((b3 & bit != 0) as usize) << 3);

                                let cram_idx = palette + color_idx;
                                let rgb = vdp.get_color(cram_idx);
                                let r = ((rgb >> 16) & 0xFF) as u8;
                                let g = ((rgb >> 8) & 0xFF) as u8;
                                let b_val = (rgb & 0xFF) as u8;

                                let px = tx * tile_size + (if h_flip { 7 - col } else { col });
                                let py = ty * tile_size + row;
                                pixels[py * img_w + px] = egui::Color32::from_rgb(r, g, b_val);
                            }
                        }
                    }
                }

                let image = egui::ColorImage {
                    size: [img_w, img_h],
                    pixels,
                };

                let texture = ctx.load_texture("vram_nt", image, egui::TextureOptions::NEAREST);

                ui.image(egui::load::SizedTexture::new(texture.id(), egui::vec2(img_w as f32 * 2.0, img_h as f32 * 2.0)));

                // Draw GG viewport rectangle overlay info
                ui.label(format!(
                    "GG Viewport: ({}, {}) to ({}, {})",
                    48, 24, 48 + 160, 24 + 144
                ));
            });
        self.show_vram_viewer = open;
    }

    pub fn draw_tile_viewer(&mut self, ctx: &egui::Context) {
        if !self.show_tile_viewer {
            return;
        }

        let mut open = self.show_tile_viewer;
        egui::Window::new("Tile Viewer")
            .open(&mut open)
            .resizable(true)
            .default_size([540.0, 560.0])
            .show(ctx, |ui| {
                let emu = match self.emu.as_ref() {
                    Some(e) => e,
                    None => {
                        ui.label("No ROM loaded");
                        return;
                    }
                };

                let vdp = &emu.bus.vdp;

                // Render all 512 tiles in a 32x16 grid
                let tiles_per_row = 32usize;
                let total_tiles = 512usize;
                let rows = total_tiles / tiles_per_row;
                let tile_size = 8usize;
                let img_w = tiles_per_row * tile_size;
                let img_h = rows * tile_size;

                // Two images: palette 0 and palette 1
                for palette_idx in 0..2u8 {
                    let palette_offset = (palette_idx as usize) * 16;

                    let mut pixels = vec![egui::Color32::BLACK; img_w * img_h];

                    for tile_idx in 0..total_tiles {
                        let tx = tile_idx % tiles_per_row;
                        let ty = tile_idx / tiles_per_row;
                        let tile_addr = tile_idx * 32;

                        for row in 0..8usize {
                            let byte_offset = tile_addr + row * 4;
                            if byte_offset + 3 >= vdp.vram.len() {
                                continue;
                            }

                            let b0 = vdp.vram[byte_offset];
                            let b1 = vdp.vram[byte_offset + 1];
                            let b2 = vdp.vram[byte_offset + 2];
                            let b3 = vdp.vram[byte_offset + 3];

                            for col in 0..8usize {
                                let bit = 1 << (7 - col);
                                let color_idx = ((b0 & bit != 0) as usize)
                                    | (((b1 & bit != 0) as usize) << 1)
                                    | (((b2 & bit != 0) as usize) << 2)
                                    | (((b3 & bit != 0) as usize) << 3);

                                let cram_idx = palette_offset + color_idx;
                                let rgb = vdp.get_color(cram_idx);
                                let r = ((rgb >> 16) & 0xFF) as u8;
                                let g = ((rgb >> 8) & 0xFF) as u8;
                                let b_val = (rgb & 0xFF) as u8;

                                let px = tx * tile_size + col;
                                let py = ty * tile_size + row;
                                pixels[py * img_w + px] = egui::Color32::from_rgb(r, g, b_val);
                            }
                        }
                    }

                    let image = egui::ColorImage {
                        size: [img_w, img_h],
                        pixels,
                    };

                    let tex_name = format!("tiles_pal{}", palette_idx);
                    let texture =
                        ctx.load_texture(tex_name, image, egui::TextureOptions::NEAREST);

                    ui.label(format!("Palette {}", palette_idx));
                    ui.image(egui::load::SizedTexture::new(
                        texture.id(),
                        egui::vec2(img_w as f32 * 2.0, img_h as f32 * 2.0),
                    ));
                    ui.separator();
                }

                // Sprite Attribute Table
                ui.collapsing("Sprite Table", |ui| {
                    let sat_base = ((vdp.registers[5] as u16 & 0x7E) << 7) as usize;

                    ui.style_mut().override_font_id =
                        Some(egui::FontId::monospace(12.0));

                    egui::Grid::new("sat_grid").num_columns(4).show(ui, |ui| {
                        ui.strong("#");
                        ui.strong("Y");
                        ui.strong("X");
                        ui.strong("Tile");
                        ui.end_row();

                        for i in 0..64usize {
                            let y = vdp.vram[sat_base + i];
                            if y == 0xD0 {
                                ui.label(format!("{}", i));
                                ui.label("-- END --");
                                ui.end_row();
                                break;
                            }
                            let x = vdp.vram[sat_base + 128 + i * 2];
                            let tile = vdp.vram[sat_base + 128 + i * 2 + 1];
                            ui.label(format!("{}", i));
                            ui.label(format!("{}", y));
                            ui.label(format!("{}", x));
                            ui.label(format!("${:02X}", tile));
                            ui.end_row();
                        }
                    });
                });
            });
        self.show_tile_viewer = open;
    }
}
