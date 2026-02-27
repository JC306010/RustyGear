use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub bios_path: Option<PathBuf>,
    pub recent_roms: Vec<PathBuf>,
    pub scale: u32,
    pub volume: f32,
    pub muted: bool,
    pub key_bindings: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut key_bindings = HashMap::new();
        key_bindings.insert("up".into(), "ArrowUp".into());
        key_bindings.insert("down".into(), "ArrowDown".into());
        key_bindings.insert("left".into(), "ArrowLeft".into());
        key_bindings.insert("right".into(), "ArrowRight".into());
        key_bindings.insert("button1".into(), "Z".into());
        key_bindings.insert("button2".into(), "X".into());
        key_bindings.insert("start".into(), "Enter".into());

        Self {
            bios_path: None,
            recent_roms: Vec::new(),
            scale: 3,
            volume: 1.0,
            muted: false,
            key_bindings,
        }
    }
}

impl Config {
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("rustygear").join("config.json"))
    }

    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(data) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&path, data);
            }
        }
    }

    pub fn add_recent_rom(&mut self, path: PathBuf) {
        self.recent_roms.retain(|p| p != &path);
        self.recent_roms.insert(0, path);
        self.recent_roms.truncate(10);
        self.save();
    }
}
