//! Settings, read at launch from `config.json` in the app's data folder and written back when
//! one is changed from the menu.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::MenuBarMetric;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Which limit the menu bar shows: "session", "weekly" or "lowest".
    pub menu_bar: MenuBarMetric,
    /// Also show each model's own limit (a Fable weekly limit) beside it.
    pub menu_bar_models: bool,
    /// Show how much of each limit is used rather than how much is left.
    pub show_used: bool,
    /// Minutes between refreshes. Anthropic allows about 30 requests an hour, shared with
    /// Claude Code, so under 5 is not accepted.
    pub refresh_minutes: u32,
    /// "glass" (Liquid Glass on macOS 26 and later) or "popover" (frosted vibrancy).
    pub material: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { menu_bar: MenuBarMetric::Session, menu_bar_models: true, show_used: false, refresh_minutes: 5, material: "popover".to_owned() }
    }
}

impl Config {
    /// Seconds between refreshes.
    pub fn refresh_seconds(&self) -> i64 {
        self.refresh_minutes.clamp(5, 120) as i64 * 60
    }
}

/// `~/Library/Application Support/nerda.bar`, or `NERDA_BAR_DIR` when set.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NERDA_BAR_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/nerda.bar")
}

/// Reads `dir/config.json`, writing the defaults there when it's missing. A file that doesn't
/// parse is left alone and the defaults are used.
pub fn load(dir: &Path) -> Config {
    let path = dir.join("config.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("nerda-bar: {} is not valid, using defaults: {e}", path.display());
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            save(dir, &config);
            config
        }
    }
}

/// Writes `dir/config.json`.
pub fn save(dir: &Path, config: &Config) {
    if std::fs::create_dir_all(dir).is_ok()
        && let Ok(text) = serde_json::to_string_pretty(config)
        && let Err(e) = std::fs::write(dir.join("config.json"), text + "\n")
    {
        eprintln!("nerda-bar: can't save settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_their_defaults() {
        let config: Config = serde_json::from_str(r#"{"menu_bar": "lowest"}"#).unwrap();
        assert_eq!(config.menu_bar, MenuBarMetric::Lowest);
        assert!(!config.show_used);
        assert_eq!(config.refresh_seconds(), 300);
    }

    #[test]
    fn refreshes_are_never_faster_than_five_minutes() {
        let config: Config = serde_json::from_str(r#"{"refresh_minutes": 1}"#).unwrap();
        assert_eq!(config.refresh_seconds(), 300);
    }
}
