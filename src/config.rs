//! User configuration: per-agent startup flags, loaded from disk.
//!
//! Issue #33: `muse --yolo`, `claude --dangerously-skip-permissions`,
//! and friends. The file is JSON at
//! `~/.config/agent-manager/config.json` (overridable for tests via
//! `AGENT_MANAGER_CONFIG`):
//!
//! ```json
//! { "agents": { "muse": { "extra_args": ["--yolo"] } } }
//! ```
//!
//! Missing file, unreadable file, or unknown keys all fall back to the
//! default (no extra args): a bad config never blocks startup. Keys are
//! matched against the spawned program name, so any supported agent can
//! carry its own flags.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::WindowAppearance;
use gpui_component::ThemeMode;
use serde::{Deserialize, Serialize};

/// Startup flags for one agent binary (keyed by program name, e.g.
/// `"muse"` or `"claude"`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Extra CLI flags appended to every spawn of this agent.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Whole-file user configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Per-agent startup flags, keyed by program name.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    /// App theme choice (issue #34).
    #[serde(default)]
    pub theme: ThemePreference,
    /// Terminal-pane font (issue #35).
    #[serde(default)]
    pub terminal: TerminalConfig,
    /// Sessions-panel width in pixels (issue #29: `[`/`]` keys adjust
    /// and persist it; narrow viewports still collapse the panel).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// History section expansion (issue #55): toggled by the History
    /// header click, `h`, or Enter on a hidden history selection, and
    /// restored on launch. Absent in older files means collapsed.
    #[serde(default)]
    pub history_expanded: bool,
}

/// Default terminal-pane font: OFL-licensed, full box-drawing + block
/// coverage, Nerd Font symbols baked in (issue #35).
pub fn default_terminal_font() -> String {
    "JetBrainsMono Nerd Font".to_string()
}

/// Default terminal font size in points (matches the historic 13.0).
pub fn default_terminal_font_size() -> f32 {
    13.0
}

/// Default sessions-panel width in pixels (issue #29: `[`/`]` keys).
pub fn default_sidebar_width() -> f32 {
    264.0
}

/// Default fallback chain behind the primary font (issue #35): emoji,
/// then CJK monospace fallbacks, then system monospace last resort.
/// A user `font_family` override replaces the head of this chain, never
/// the emoji/CJK tail.
pub fn default_fallback_fonts() -> Vec<String> {
    vec![
        "Apple Color Emoji".to_string(),
        "Noto Color Emoji".to_string(),
        "Noto Sans Mono CJK SC".to_string(),
        "Noto Sans Mono CJK JP".to_string(),
        "Hiragino Kaku Gothic ProN".to_string(),
        "PingFang SC".to_string(),
        "Menlo".to_string(),
        "DejaVu Sans Mono".to_string(),
    ]
}

/// Terminal-pane font setting (issue #35). No `Eq`: `font_size` is a
/// float (config equality is still exact via `PartialEq` in tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalConfig {
    /// Primary monospace font family. Default is the recommended
    /// JetBrainsMono Nerd Font (see README for install); set this to any
    /// installed patched font (FiraCode, Hack, Iosevka, …) to override.
    #[serde(default = "default_terminal_font")]
    pub font_family: String,
    /// Font size in points.
    #[serde(default = "default_terminal_font_size")]
    pub font_size: f32,
    /// Ordered fallback families behind `font_family`.
    #[serde(default = "default_fallback_fonts")]
    pub fallback_fonts: Vec<String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_family: default_terminal_font(),
            font_size: default_terminal_font_size(),
            fallback_fonts: default_fallback_fonts(),
        }
    }
}

impl TerminalConfig {
    /// Ordered font stack: the (possibly overridden) primary first, then
    /// the emoji/CJK/system fallbacks with any duplicate of the primary
    /// removed. The override replaces the head, never the tail.
    pub fn font_stack(&self) -> Vec<String> {
        let mut stack = vec![self.font_family.clone()];
        for fallback in &self.fallback_fonts {
            if *fallback != self.font_family && !stack.contains(fallback) {
                stack.push(fallback.clone());
            }
        }
        stack
    }
}

/// App theme choice (issue #34): explicit dark/light, or follow the OS.
/// Serialized lowercase (`"dark"`, `"light"`, `"system"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the OS appearance (dark when unknown).
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePreference {
    /// Next choice in the `t`-key cycle: dark → light → system → dark.
    pub fn cycle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::System,
            Self::System => Self::Dark,
        }
    }

    /// Config-file label (`t`-key status flash, README).
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    /// Resolve to a concrete component theme. `appearance` is the live
    /// window appearance; `None` (headless/tests) resolves system to dark,
    /// preserving the pre-#34 default.
    pub fn theme_mode(self, appearance: Option<WindowAppearance>) -> ThemeMode {
        match self {
            Self::Dark => ThemeMode::Dark,
            Self::Light => ThemeMode::Light,
            Self::System => match appearance {
                Some(WindowAppearance::Dark | WindowAppearance::VibrantDark) => ThemeMode::Dark,
                Some(_) => ThemeMode::Light,
                None => ThemeMode::Dark,
            },
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            theme: ThemePreference::default(),
            terminal: TerminalConfig::default(),
            sidebar_width: default_sidebar_width(),
            history_expanded: false,
        }
    }
}

impl Config {
    /// Path of the config file: `$AGENT_MANAGER_CONFIG` when set (tests),
    /// otherwise `~/.config/agent-manager/config.json`.
    pub fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("AGENT_MANAGER_CONFIG") {
            return PathBuf::from(path);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("agent-manager")
            .join("config.json")
    }

    /// Load from [`Self::config_path`]; any failure (missing file,
    /// unreadable, malformed JSON) yields the default config.
    pub fn load() -> Self {
        Self::load_from(&Self::config_path())
    }

    /// Load from an explicit path; same forgiving policy as [`Self::load`].
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Persist to [`Self::config_path`], creating parent directories.
    /// Returns the IO/serialization error so callers can surface it.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// Extra startup flags for `agent` (the spawned program name).
    /// Unknown agents get none.
    pub fn extra_args_for(&self, agent: &str) -> Vec<String> {
        self.agents
            .get(agent)
            .map(|a| a.extra_args.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped_env(path: &std::path::Path) -> Option<String> {
        // Single test in this module touches AGENT_MANAGER_CONFIG, so no
        // cross-test interference: nothing else reads this variable.
        let old = std::env::var("AGENT_MANAGER_CONFIG").ok();
        std::env::set_var("AGENT_MANAGER_CONFIG", path);
        old
    }

    #[test]
    fn default_config_carries_no_extra_args() {
        let cfg = Config::default();
        assert!(cfg.extra_args_for("muse").is_empty());
        assert!(cfg.extra_args_for("claude").is_empty());
        assert_eq!(cfg.theme, ThemePreference::System);
    }

    #[test]
    fn sidebar_width_defaults_and_survives_json() {
        // Issue #29: the `[`/`]` panel width persists in the same local
        // config file as the other comfort settings.
        assert_eq!(Config::default().sidebar_width, 264.0);
        let partial: Config = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        assert_eq!(partial.sidebar_width, 264.0);
        let mut cfg = Config {
            sidebar_width: 300.0,
            ..Default::default()
        };
        // A font-size change (the `+`/`-` keys) survives the same trip.
        cfg.terminal.font_size = 17.0;
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.sidebar_width, 300.0);
        assert_eq!(back.terminal.font_size, 17.0);
    }

    #[test]
    fn history_expansion_defaults_collapsed_and_survives_json() {
        // Issue #55: expansion persists in the same local config file;
        // files written before the key existed (or with it absent)
        // still load as collapsed.
        assert!(!Config::default().history_expanded);
        let partial: Config = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        assert!(!partial.history_expanded);
        let cfg = Config {
            history_expanded: true,
            ..Default::default()
        };
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(back.history_expanded);
    }

    #[test]
    fn terminal_font_stack_heads_primary_and_keeps_fallbacks() {
        // Issue #35: default head is JetBrainsMono Nerd Font with the
        // emoji/CJK tail behind it.
        let cfg = TerminalConfig::default();
        let stack = cfg.font_stack();
        assert_eq!(stack[0], "JetBrainsMono Nerd Font");
        assert!(stack.contains(&"Apple Color Emoji".to_string()));
        assert!(stack.contains(&"Noto Sans Mono CJK SC".to_string()));
        assert_eq!(stack.last().unwrap(), "DejaVu Sans Mono");
        assert_eq!(cfg.font_size, 13.0);
        // A user override replaces the head, never the tail.
        let custom = TerminalConfig {
            font_family: "FiraCode Nerd Font".to_string(),
            ..TerminalConfig::default()
        };
        let stack = custom.font_stack();
        assert_eq!(stack[0], "FiraCode Nerd Font");
        assert!(stack.contains(&"Apple Color Emoji".to_string()));
        assert!(stack.contains(&"Noto Sans Mono CJK SC".to_string()));
        assert_eq!(
            stack.iter().filter(|f| *f == "FiraCode Nerd Font").count(),
            1
        );
        // Partial JSON keeps defaults for missing keys.
        let partial: Config = serde_json::from_str(r#"{"terminal": {"font_size": 15.0}}"#).unwrap();
        assert_eq!(partial.terminal.font_family, "JetBrainsMono Nerd Font");
        assert_eq!(partial.terminal.font_size, 15.0);
        assert_eq!(partial.terminal.fallback_fonts, default_fallback_fonts());
    }

    #[test]
    fn theme_choice_cycles_parses_and_resolves() {
        // Issue #34: dark → light → system → dark.
        use gpui::WindowAppearance;
        use gpui_component::ThemeMode;
        assert_eq!(ThemePreference::Dark.cycle(), ThemePreference::Light);
        assert_eq!(ThemePreference::Light.cycle(), ThemePreference::System);
        assert_eq!(ThemePreference::System.cycle(), ThemePreference::Dark);
        assert_eq!(ThemePreference::Dark.label(), "dark",);
        // Lowercase config labels parse; explicit choices ignore the OS.
        let parsed: Config = serde_json::from_str(r#"{"theme": "light"}"#).unwrap();
        assert_eq!(parsed.theme, ThemePreference::Light);
        assert_eq!(
            ThemePreference::Dark.theme_mode(Some(WindowAppearance::Light)),
            ThemeMode::Dark
        );
        assert_eq!(
            ThemePreference::Light.theme_mode(Some(WindowAppearance::Dark)),
            ThemeMode::Light
        );
        // System follows the OS; unknown stays on the historic dark.
        assert_eq!(
            ThemePreference::System.theme_mode(Some(WindowAppearance::VibrantDark)),
            ThemeMode::Dark
        );
        assert_eq!(
            ThemePreference::System.theme_mode(Some(WindowAppearance::Light)),
            ThemeMode::Light
        );
        assert_eq!(ThemePreference::System.theme_mode(None), ThemeMode::Dark);
    }

    #[test]
    fn extra_args_round_trip_through_json() {
        let mut cfg = Config::default();
        cfg.agents.insert(
            "muse".to_string(),
            AgentConfig {
                extra_args: vec!["--yolo".to_string()],
            },
        );
        let text = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(back.extra_args_for("muse"), vec!["--yolo".to_string()]);
        // Unknown keys are ignored, not fatal.
        let lenient: Config = serde_json::from_str(r#"{"agents": {}, "future": 1}"#).unwrap();
        assert_eq!(lenient, Config::default());
    }

    #[test]
    fn missing_or_malformed_file_falls_back_to_default() {
        let dir = std::env::temp_dir().join("agent-manager-cfg-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Config::load_from(&dir.join("nope.json")), Config::default());
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert_eq!(Config::load_from(&bad), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_preserve_extra_args() {
        let dir = std::env::temp_dir().join("agent-manager-cfg-test-save");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let old = scoped_env(&path);
        let mut cfg = Config::default();
        cfg.agents.insert(
            "claude".to_string(),
            AgentConfig {
                extra_args: vec!["--dangerously-skip-permissions".to_string()],
            },
        );
        cfg.save().unwrap();
        assert_eq!(Config::load(), cfg);
        if let Some(v) = old {
            std::env::set_var("AGENT_MANAGER_CONFIG", v);
        } else {
            std::env::remove_var("AGENT_MANAGER_CONFIG");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
