//! Local-only comfort preferences: terminal font size + panel width.
//!
//! Plain JSON under the platform data dir (`agent-manager/prefs.json`);
//! no account, no sync, no network. Missing or corrupt files degrade to
//! [`Prefs::default()`], so the app always starts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default terminal font size in points.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// Default sessions panel width in pixels.
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 264.0;

const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 32.0;
const MIN_SIDEBAR_WIDTH: f32 = 160.0;
const MAX_SIDEBAR_WIDTH: f32 = 480.0;

/// Comfort preferences, persisted as local JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    pub font_size: f32,
    pub sidebar_width: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl Prefs {
    /// Clamp both fields into their sane ranges.
    pub fn clamped(mut self) -> Self {
        self.font_size = self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.sidebar_width = self
            .sidebar_width
            .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        self
    }

    /// Load from the default path, defaulting on any failure.
    pub fn load() -> Self {
        Self::load_from(&prefs_path())
    }

    /// Load from `path`, defaulting on any failure (missing/corrupt).
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Prefs>(&text).ok())
            .map(Prefs::clamped)
            .unwrap_or_default()
    }

    /// Save to the default path (creating parent dirs). Best effort: the
    /// caller flashes the change regardless; a failed save just means the
    /// next start falls back to defaults. Production-only: unit tests
    /// must never rewrite the developer's live prefs file (the roundtrip
    /// is covered through `save_to` with temp paths instead).
    #[cfg(not(test))]
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&prefs_path())
    }

    /// Save to `path` (creating parent dirs).
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }
}

/// Platform data dir for local-only state (`agent-manager` subdir).
pub fn data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("agent-manager")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/agent-manager")
    } else {
        PathBuf::from(".local/share/agent-manager")
    }
}

/// Default prefs file path.
pub fn prefs_path() -> PathBuf {
    data_dir().join("prefs.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-manager-prefs-test-{}-{name}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn font_change_survives_a_save_load_roundtrip() {
        let path = tmp_path("roundtrip.json");
        let prefs = Prefs {
            font_size: 17.0,
            sidebar_width: 300.0,
        };
        prefs.save_to(&path).unwrap();
        assert_eq!(Prefs::load_from(&path), prefs);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_or_corrupt_prefs_degrade_to_defaults() {
        assert_eq!(
            Prefs::load_from(&tmp_path("missing.json")),
            Prefs::default()
        );
        let path = tmp_path("corrupt.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json{{{").unwrap();
        assert_eq!(Prefs::load_from(&path), Prefs::default());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn out_of_range_values_clamp_on_load() {
        let path = tmp_path("clamp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"font_size": 200.0, "sidebar_width": 10.0}"#).unwrap();
        let prefs = Prefs::load_from(&path);
        assert_eq!(prefs.font_size, MAX_FONT_SIZE);
        assert_eq!(prefs.sidebar_width, MIN_SIDEBAR_WIDTH);
        std::fs::remove_file(&path).ok();
    }
}
