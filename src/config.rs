//! Settings, read at launch from `config.json` in the app's data folder and written back when
//! one is changed from the menu.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{MenuBarMetric, Provider};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Which limit the menu bar shows: "session", "weekly" or "lowest".
    pub menu_bar: MenuBarMetric,
    /// Also show each model's own limit (a Fable weekly limit) beside it.
    pub menu_bar_models: bool,
    /// Show how much of each limit is used rather than how much is left.
    pub show_used: bool,
    /// What Codex's menu bar item shows.
    pub codex: ProviderBar,
    /// Whether Claude has an item in the menu bar, and a tab in the panel. An agent with
    /// neither is not asked for anything.
    pub claude_in_menu_bar: bool,
    pub claude_tab: bool,
    pub codex_in_menu_bar: bool,
    pub codex_tab: bool,
    /// Whether DeepSeek's balance has an item in the menu bar. It was first written as
    /// `deepseek_menu_bar`, which still reads.
    #[serde(alias = "deepseek_menu_bar")]
    pub deepseek_in_menu_bar: bool,
    pub deepseek_tab: bool,
    /// DeepSeek's balance reads as a warning under this amount, in its own currency; 0 never
    /// warns.
    pub deepseek_warn_below: f64,
    /// Minutes between refreshes. Anthropic allows about 30 requests an hour, shared with
    /// Claude Code, so under 5 is not accepted.
    pub refresh_minutes: u32,
    /// "glass" (Liquid Glass on macOS 26 and later) or "popover" (frosted vibrancy).
    pub material: String,
    /// The CPU's own menu bar item and tab.
    pub cpu: Module,
    /// Memory's own menu bar item and tab.
    pub memory: Module,
    pub disk: Module,
    pub network: Module,
    /// On a Mac without a battery the tab never shows, whatever this says.
    pub battery: Module,
    pub gpu: Module,
    /// The Sleep tab, and how it keeps this Mac awake.
    pub sleep: SleepConfig,
}

/// The Sleep tab: whether it shows, whether a keep-awake has a menu bar item while it runs,
/// and what the next one keeps on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SleepConfig {
    pub tab: bool,
    /// A cup in the menu bar while this Mac is kept awake, with the time left.
    pub menu_bar: bool,
    /// Keep the display on too, not only the Mac.
    pub display: bool,
    /// Also keep it awake with the lid closed, which asks for a password.
    pub lid: bool,
}

impl Default for SleepConfig {
    fn default() -> Self {
        Self { tab: true, menu_bar: true, display: true, lid: false }
    }
}

/// What a provider's menu bar item shows, for a provider other than Claude (whose settings
/// keep the top-level names they had first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderBar {
    pub menu_bar: MenuBarMetric,
    pub show_used: bool,
    /// Also show each model's own limit beside it.
    pub models: bool,
}

/// How one of this Mac's measures shows: its menu bar item, and its tab in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Module {
    pub menu_bar: MenuStyle,
    pub tab: bool,
}

impl Default for Module {
    fn default() -> Self {
        Self { menu_bar: MenuStyle::Off, tab: true }
    }
}

/// How a measure reads in the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuStyle {
    /// No item.
    Off,
    /// Its name and a percentage: "CPU 8%".
    Text,
    /// A ring filled to the share, and the percentage.
    Ring,
    /// The last half minute as bars, and the percentage.
    Graph,
}

impl MenuStyle {
    pub const ALL: [MenuStyle; 4] = [MenuStyle::Off, MenuStyle::Text, MenuStyle::Ring, MenuStyle::Graph];

    pub fn label(self) -> &'static str {
        match self {
            MenuStyle::Off => "Off",
            MenuStyle::Text => "Text",
            MenuStyle::Ring => "Ring",
            MenuStyle::Graph => "Graph",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            menu_bar: MenuBarMetric::Session,
            menu_bar_models: true,
            show_used: false,
            codex: ProviderBar::default(),
            claude_in_menu_bar: true,
            claude_tab: true,
            codex_in_menu_bar: true,
            codex_tab: true,
            deepseek_in_menu_bar: true,
            deepseek_tab: true,
            deepseek_warn_below: 0.0,
            refresh_minutes: 5,
            material: "popover".to_owned(),
            cpu: Module::default(),
            memory: Module::default(),
            disk: Module::default(),
            network: Module::default(),
            battery: Module::default(),
            gpu: Module::default(),
            sleep: SleepConfig::default(),
        }
    }
}

/// The refresh intervals the settings offer, in minutes.
pub const REFRESH_CHOICES: [u32; 4] = [5, 10, 15, 30];

impl Config {
    /// Seconds between refreshes.
    pub fn refresh_seconds(&self) -> i64 {
        self.refresh_minutes.clamp(5, 120) as i64 * 60
    }

    /// Whether `provider` has an item in the menu bar.
    pub fn in_menu_bar(&self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.claude_in_menu_bar,
            Provider::Codex => self.codex_in_menu_bar,
            Provider::DeepSeek => self.deepseek_in_menu_bar,
        }
    }

    /// Whether `provider` has a tab in the panel.
    pub fn has_tab(&self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.claude_tab,
            Provider::Codex => self.codex_tab,
            Provider::DeepSeek => self.deepseek_tab,
        }
    }

    /// Whether `provider` shows anywhere: one that doesn't is not read.
    pub fn shows(&self, provider: Provider) -> bool {
        self.in_menu_bar(provider) || self.has_tab(provider)
    }

    pub fn set_in_menu_bar(&mut self, provider: Provider, on: bool) {
        *match provider {
            Provider::Claude => &mut self.claude_in_menu_bar,
            Provider::Codex => &mut self.codex_in_menu_bar,
            Provider::DeepSeek => &mut self.deepseek_in_menu_bar,
        } = on;
    }

    pub fn set_tab(&mut self, provider: Provider, on: bool) {
        *match provider {
            Provider::Claude => &mut self.claude_tab,
            Provider::Codex => &mut self.codex_tab,
            Provider::DeepSeek => &mut self.deepseek_tab,
        } = on;
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
/// parse is kept beside it as `config.invalid.json` and the defaults are used: the next
/// change made in the panel writes `config.json` whole, and would otherwise be the end of
/// what was typed there.
pub fn load(dir: &Path) -> Config {
    match read(dir) {
        Some(Ok(config)) => config,
        Some(Err(reason)) => {
            eprintln!("nerda-bar: {} is not valid, using defaults (kept as config.invalid.json): {reason}", dir.join("config.json").display());
            Config::default()
        }
        None => {
            let config = Config::default();
            save(dir, &config);
            config
        }
    }
}

/// `dir/config.json` as it is now: `None` when there is none, `Err` when it doesn't parse, in
/// which case a copy is kept as `config.invalid.json`.
pub fn read(dir: &Path) -> Option<Result<Config, String>> {
    let text = std::fs::read_to_string(dir.join("config.json")).ok()?;
    Some(serde_json::from_str(&text).map_err(|e| {
        let _ = write_file(dir, "config.invalid.json", &text);
        e.to_string()
    }))
}

/// When `dir/config.json` was last written, by this app or by hand.
pub fn modified(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(dir.join("config.json")).and_then(|meta| meta.modified()).ok()
}

/// Writes `dir/config.json`.
pub fn save(dir: &Path, config: &Config) {
    if let Ok(text) = serde_json::to_string_pretty(config)
        && let Err(e) = write_file(dir, "config.json", &(text + "\n"))
    {
        eprintln!("nerda-bar: can't save settings: {e}");
    }
}

/// Writes `dir/name` whole or not at all: the text goes to a file beside it, which then takes
/// its name. A crash or a power cut mid-write leaves the earlier file, never half of this one.
pub fn write_file(dir: &Path, name: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let partial = dir.join(format!(".{name}.part"));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, dir.join(name))
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
        assert_eq!(config.cpu, Module { menu_bar: MenuStyle::Off, tab: true });
    }

    #[test]
    fn a_module_reads_with_its_missing_fields_defaulted() {
        let config: Config = serde_json::from_str(r#"{"cpu": {"menu_bar": "ring"}}"#).unwrap();
        assert_eq!(config.cpu, Module { menu_bar: MenuStyle::Ring, tab: true });
        assert_eq!(config.memory, Module::default());
    }

    #[test]
    fn refreshes_are_never_faster_than_five_minutes() {
        let config: Config = serde_json::from_str(r#"{"refresh_minutes": 1}"#).unwrap();
        assert_eq!(config.refresh_seconds(), 300);
    }

    /// The settings file of an install from before every agent could be switched: each key
    /// keeps its meaning, and the new ones take their defaults.
    #[test]
    fn an_earlier_versions_file_reads_as_it_was() {
        let text = r#"{"menu_bar": "session", "menu_bar_models": true, "show_used": false, "codex": {"menu_bar": "session", "show_used": false},
            "deepseek_menu_bar": false, "refresh_minutes": 5, "material": "popover", "cpu": {"menu_bar": "off", "tab": true},
            "memory": {"menu_bar": "text", "tab": true}, "sleep": {"tab": true, "menu_bar": true, "display": true, "lid": false}}"#;
        let config: Config = serde_json::from_str(text).unwrap();
        assert!(!config.deepseek_in_menu_bar, "DeepSeek's item stays off under its first name");
        assert!(config.claude_in_menu_bar && config.codex_in_menu_bar && config.claude_tab && config.codex_tab && config.deepseek_tab);
        assert_eq!(config.memory.menu_bar, MenuStyle::Text);
        assert!(!config.codex.models);
        // Written back, it reads the same.
        let again: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(again, config);
    }

    #[test]
    fn a_file_that_does_not_parse_is_kept_and_not_lost() {
        let dir = std::env::temp_dir().join(format!("nerda-bar-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let typed = r#"{"menu_bar": "weekly", "cpu": {"menu_bar": "Ring"},}"#;
        std::fs::write(dir.join("config.json"), typed).unwrap();
        assert_eq!(load(&dir), Config::default());
        save(&dir, &Config::default());
        assert_eq!(std::fs::read_to_string(dir.join("config.invalid.json")).unwrap(), typed, "what was typed is still there");
        assert_eq!(read(&dir), Some(Ok(Config::default())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_agent_that_shows_nowhere_is_not_read() {
        let mut config = Config::default();
        config.set_in_menu_bar(Provider::Codex, false);
        assert!(config.shows(Provider::Codex), "its tab still shows it");
        config.set_tab(Provider::Codex, false);
        assert!(!config.shows(Provider::Codex) && config.shows(Provider::Claude));
    }
}
