//! Persisted user configuration (`%APPDATA%\Glint\config.json`). Schema: docs/ARCHITECTURE.md.

use std::path::{Path, PathBuf};

use glint_translate::EngineConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Config {
    pub general: General,
    pub hotkeys: Hotkeys,
    pub ocr: Ocr,
    pub translate: Translate,
    pub ui: Ui,
    pub overlay: Overlay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct General {
    pub ui_language: String,
    pub autostart: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            ui_language: "zh-CN".into(),
            autostart: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    pub translate: String,
    pub ocr: String,
    pub silent_ocr: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            translate: "Alt+Q".into(),
            ocr: "Alt+W".into(),
            silent_ocr: "Alt+Shift+W".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ocr {
    /// "paddle" | "windows"
    pub engine: String,
    pub merge_lines: bool,
    pub idle_unload_secs: u64,
}

impl Default for Ocr {
    fn default() -> Self {
        Self {
            engine: "paddle".into(),
            merge_lines: true,
            idle_unload_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Translate {
    pub source_lang: String,
    pub target_lang: String,
    pub secondary_lang: String,
    pub engines: Vec<EngineConfig>,
}

impl Default for Translate {
    fn default() -> Self {
        Self {
            source_lang: "auto".into(),
            target_lang: "zh-Hans".into(),
            secondary_lang: "en".into(),
            engines: glint_translate::default_engines(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ui {
    /// "system" | "light" | "dark"
    pub theme: String,
    pub font_size: u32,
    pub accent: String,
    pub close_on_blur: bool,
    pub popup_idle_destroy_secs: u64,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            font_size: 14,
            accent: "#1E6AC5".into(),
            close_on_blur: true,
            popup_idle_destroy_secs: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Overlay {
    pub snap_windows: bool,
    pub magnifier: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            snap_windows: true,
            magnifier: true,
        }
    }
}

impl Ui {
    /// Accent as 0xRRGGBB; falls back to the default on malformed input.
    pub fn accent_rgb(&self) -> u32 {
        let hex = self.accent.trim().trim_start_matches('#');
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return v;
            }
        }
        0x1E6AC5
    }
}

pub fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("config.json")
}

/// Load config, falling back to defaults when missing or unreadable (a broken file is kept
/// aside as `config.json.bak` so the user's keys are not silently lost).
pub fn load(path: &Path) -> Config {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return Config::default(),
    };
    match serde_json::from_str::<Config>(&raw) {
        Ok(mut cfg) => {
            merge_new_engines(&mut cfg);
            cfg
        }
        Err(e) => {
            log::warn!("config parse failed ({e}); using defaults");
            let _ = std::fs::copy(path, path.with_extension("json.bak"));
            Config::default()
        }
    }
}

/// Write atomically (temp file + rename) so a crash never leaves a truncated config.
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Engines added in newer versions appear (disabled) in existing configs.
fn merge_new_engines(cfg: &mut Config) {
    for def in glint_translate::default_engines() {
        if !cfg.translate.engines.iter().any(|e| e.kind == def.kind) {
            cfg.translate.engines.push(EngineConfig {
                enabled: false,
                ..def
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_fills_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"ui":{"fontSize":16}}"#).unwrap();
        assert_eq!(cfg.ui.font_size, 16);
        assert_eq!(cfg.ui.theme, "system");
        assert_eq!(cfg.hotkeys.translate, "Alt+Q");
    }

    #[test]
    fn accent_parsing() {
        let mut ui = Ui {
            accent: "#ff8800".into(),
            ..Ui::default()
        };
        assert_eq!(ui.accent_rgb(), 0xFF8800);
        ui.accent = "nope".into();
        assert_eq!(ui.accent_rgb(), 0x1E6AC5);
    }
}
