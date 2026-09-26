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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Per-agent startup flags, keyed by program name.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
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
    /// First real caller is theme persistence (issue #34).
    #[allow(dead_code)]
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
