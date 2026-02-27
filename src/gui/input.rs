use eframe::egui;

use super::app::RustyGearApp;

pub fn key_name_to_egui(name: &str) -> Option<egui::Key> {
    match name {
        "ArrowUp" => Some(egui::Key::ArrowUp),
        "ArrowDown" => Some(egui::Key::ArrowDown),
        "ArrowLeft" => Some(egui::Key::ArrowLeft),
        "ArrowRight" => Some(egui::Key::ArrowRight),
        "Enter" => Some(egui::Key::Enter),
        "Space" => Some(egui::Key::Space),
        "Tab" => Some(egui::Key::Tab),
        "Backspace" => Some(egui::Key::Backspace),
        _ => {
            // Single letter keys A-Z
            if name.len() == 1 {
                let ch = name.chars().next().unwrap().to_ascii_uppercase();
                if ch.is_ascii_alphabetic() {
                    return egui::Key::from_name(&ch.to_string());
                }
            }
            // Number keys 0-9
            if name.len() == 1 {
                let ch = name.chars().next().unwrap();
                if ch.is_ascii_digit() {
                    return egui::Key::from_name(name);
                }
            }
            None
        }
    }
}

pub fn egui_key_to_name(key: egui::Key) -> String {
    format!("{:?}", key)
}

impl RustyGearApp {
    pub fn handle_input(&mut self, ctx: &egui::Context) {
        let emu = match self.emu.as_mut() {
            Some(e) => e,
            None => return,
        };

        let actions = ["up", "down", "left", "right", "button1", "button2", "start"];

        for action in &actions {
            let key_name = match self.config.key_bindings.get(*action) {
                Some(name) => name.clone(),
                None => continue,
            };
            let key = match key_name_to_egui(&key_name) {
                Some(k) => k,
                None => continue,
            };

            let pressed = ctx.input(|i| i.key_down(key));

            match *action {
                "up" => emu.bus.input.up = pressed,
                "down" => emu.bus.input.down = pressed,
                "left" => emu.bus.input.left = pressed,
                "right" => emu.bus.input.right = pressed,
                "button1" => emu.bus.input.button1 = pressed,
                "button2" => emu.bus.input.button2 = pressed,
                "start" => emu.bus.input.start = pressed,
                _ => {}
            }
        }

        // Hotkeys
        ctx.input(|i| {
            if i.key_pressed(egui::Key::P) && !self.show_input_settings {
                self.paused = !self.paused;
            }
            if i.key_pressed(egui::Key::N) && self.paused {
                self.frame_step = true;
            }
            if i.key_pressed(egui::Key::M) && !self.show_input_settings {
                self.config.muted = !self.config.muted;
                self.config.save();
            }
        });
    }

    pub fn draw_input_settings(&mut self, ctx: &egui::Context) {
        if !self.show_input_settings {
            return;
        }

        let mut open = self.show_input_settings;
        egui::Window::new("Input Bindings")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let actions = [
                    ("up", "D-Pad Up"),
                    ("down", "D-Pad Down"),
                    ("left", "D-Pad Left"),
                    ("right", "D-Pad Right"),
                    ("button1", "Button 1"),
                    ("button2", "Button 2"),
                    ("start", "Start"),
                ];

                egui::Grid::new("input_grid").num_columns(3).show(ui, |ui| {
                    ui.strong("Action");
                    ui.strong("Key");
                    ui.strong("");
                    ui.end_row();

                    for (action, label) in &actions {
                        ui.label(*label);

                        let current_key = self
                            .config
                            .key_bindings
                            .get(*action)
                            .cloned()
                            .unwrap_or_else(|| "???".into());

                        let is_awaiting = self.awaiting_key_for.as_deref() == Some(*action);

                        if is_awaiting {
                            ui.label("Press a key...");

                            // Check for any key press
                            let mut new_key = None;
                            ctx.input(|i| {
                                for event in &i.events {
                                    if let egui::Event::Key { key, pressed: true, .. } = event {
                                        new_key = Some(*key);
                                    }
                                }
                            });

                            if let Some(key) = new_key {
                                self.config
                                    .key_bindings
                                    .insert(action.to_string(), egui_key_to_name(key));
                                self.config.save();
                                self.awaiting_key_for = None;
                            }
                        } else {
                            ui.label(&current_key);
                        }

                        if !is_awaiting {
                            if ui.button("Bind").clicked() {
                                self.awaiting_key_for = Some(action.to_string());
                            }
                        } else if ui.button("Cancel").clicked() {
                            self.awaiting_key_for = None;
                        }

                        ui.end_row();
                    }
                });

                ui.separator();
                if ui.button("Reset to Defaults").clicked() {
                    let defaults = super::config::Config::default();
                    self.config.key_bindings = defaults.key_bindings;
                    self.config.save();
                    self.awaiting_key_for = None;
                }
            });
        self.show_input_settings = open;
    }
}
