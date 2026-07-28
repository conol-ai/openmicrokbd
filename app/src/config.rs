//! Persisted app config: what each macropad key does on the host.
//!
//! The firmware sends fixed usages (F13..F24, see ../fw/src/main.rs); the
//! *meaning* of a key lives here, per host. Stored as JSON under the OS
//! config dir (e.g. ~/Library/Application Support/OpenMicro/config.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The 12 distinct usages the firmware emits, in order F13..F24.
/// (13 physical keys; keys 11+12 share the 2U keycap and both send F23.)
pub const KEY_COUNT: usize = 12;

/// Physical position of each F-key on the pad, for UI labels.
pub const KEY_LABELS: [&str; KEY_COUNT] = [
    "Top row · left of centre",
    "Top row · right of centre",
    "Row 2 · first column",
    "Row 2 · second column",
    "Row 2 · third column",
    "Row 2 · fourth column",
    "Row 3 · first column",
    "Row 3 · second column",
    "Row 3 · third column",
    "Row 3 · fourth column",
    "Bottom row · 2U keycap",
    "Bottom row · far right",
];

/// Short name of each binding, as printed on the pad's key map.
pub const KEY_TITLES: [&str; KEY_COUNT] = [
    "Key 1", "Key 2", "Key 3", "Key 4", "Key 5", "Key 6", "Key 7", "Key 8", "Key 9", "Key 10",
    "Keys 11 + 12", "Key 13",
];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BindKind {
    #[default]
    None,
    /// Run a shell command (`sh -c` / `cmd /C`).
    Run,
    /// Open a URL, file, or application with the OS default handler.
    Open,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Binding {
    pub kind: BindKind,
    pub arg: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    /// Index 0 = F13 … index 11 = F24.
    pub bindings: Vec<Binding>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            bindings: vec![Binding::default(); KEY_COUNT],
        }
    }
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("OpenMicro").join("config.json"))
}

pub fn load() -> AppConfig {
    let Some(path) = config_path() else {
        return AppConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    let mut cfg: AppConfig = serde_json::from_str(&text).unwrap_or_default();
    cfg.bindings.resize(KEY_COUNT, Binding::default());
    cfg
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let path = config_path().ok_or("no config dir on this platform")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}
