//! Supported agent CLIs and per-agent extra startup flags.
//!
//! The app launches one real agent CLI per run (`muse` by default; also
//! `claude`, `codex`, `opencode`). Each CLI accepts different startup
//! options (`--yolo` for `muse`, `--dangerously-skip-permissions` for
//! `claude`, ...), so extra flags are stored per agent id in a plain JSON
//! file and appended to that agent's launch command. A missing or invalid
//! file degrades to defaults (no extra flags), matching the provider's
//! degraded-empty convention.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

/// An agent CLI the app can launch. The binary name is the id itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    /// `muse` (default).
    #[default]
    Muse,
    /// `claude`.
    Claude,
    /// `codex`.
    Codex,
    /// `opencode`.
    Opencode,
}

impl Agent {
    /// Every launchable agent, in switcher order.
    pub fn all() -> &'static [Agent] {
        &[Agent::Muse, Agent::Claude, Agent::Codex, Agent::Opencode]
    }

    /// CLI id, which is also the binary name.
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Muse => "muse",
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Opencode => "opencode",
        }
    }

    /// Parse an id from config or UI text. Case-insensitive with
    /// surrounding whitespace tolerated; unknown ids are `None` so
    /// callers keep the current agent.
    pub fn parse(s: &str) -> Option<Agent> {
        match s.trim().to_ascii_lowercase().as_str() {
            "muse" => Some(Agent::Muse),
            "claude" => Some(Agent::Claude),
            "codex" => Some(Agent::Codex),
            "opencode" => Some(Agent::Opencode),
            _ => None,
        }
    }

    /// The agent after this one in [`Agent::all`] order (wraps around).
    pub fn next(self) -> Agent {
        let all = Agent::all();
        let i = all.iter().position(|a| *a == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    /// One commonly used startup flag, shown as an editor hint.
    /// Empty when no canonical example exists for the agent.
    pub fn suggested_flags(self) -> &'static str {
        match self {
            Agent::Muse => "--yolo",
            Agent::Claude => "--dangerously-skip-permissions",
            Agent::Codex => "",
            Agent::Opencode => "",
        }
    }
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-agent extra CLI flags plus the selected agent. Serialized as plain
/// JSON (`{"active_agent": "muse", "extra_args": {"muse": ["--yolo"]}}`);
/// unknown map keys are kept verbatim so a newer config never loses flags.
/// Deserialization is lenient: an unknown `active_agent` id falls back to
/// the default agent (keeping any stored flags) instead of failing the
/// whole load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct AgentConfig {
    /// Agent used for the next launch.
    pub active_agent: Agent,
    /// Extra argv entries per agent id (see [`Agent::as_str`]).
    pub extra_args: HashMap<String, Vec<String>>,
}

/// Serde helper: `active_agent` arrives as a plain string so an unknown
/// id can degrade to the default (see [`Agent::parse`]).
#[derive(Deserialize)]
struct RawAgentConfig {
    #[serde(default)]
    active_agent: Option<String>,
    #[serde(default)]
    extra_args: Option<HashMap<String, Vec<String>>>,
}

impl<'de> Deserialize<'de> for AgentConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawAgentConfig::deserialize(deserializer)?;
        Ok(Self {
            active_agent: raw
                .active_agent
                .as_deref()
                .and_then(Agent::parse)
                .unwrap_or_default(),
            extra_args: raw.extra_args.unwrap_or_default(),
        })
    }
}

impl AgentConfig {
    /// Flags currently stored for `agent` (empty when none).
    pub fn extra_for(&self, agent: Agent) -> &[String] {
        self.extra_args
            .get(agent.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Flags for `agent` as editable text (shell-quoted where needed).
    pub fn extra_text(&self, agent: Agent) -> String {
        join_shell_words(self.extra_for(agent))
    }

    /// Replace `agent`'s flags. Empty lists drop the key so the file
    /// stays free of dead entries.
    pub fn set_extra(&mut self, agent: Agent, args: Vec<String>) {
        if args.is_empty() {
            self.extra_args.remove(agent.as_str());
        } else {
            self.extra_args.insert(agent.as_str().to_string(), args);
        }
    }

    /// Parse `raw` shell-style words and store them for `agent`.
    /// Returns the normalized text. A parse failure leaves the stored
    /// flags untouched.
    pub fn set_extra_raw(&mut self, agent: Agent, raw: &str) -> Result<String, String> {
        let args = split_shell_words(raw)?;
        self.set_extra(agent, args);
        Ok(self.extra_text(agent))
    }

    /// Full launch command for `agent`: its binary plus stored flags.
    pub fn launch_command(&self, agent: Agent) -> (String, Vec<String>) {
        (agent.as_str().to_string(), self.extra_for(agent).to_vec())
    }

    /// One-line summary used by status flashes, e.g.
    /// `claude --dangerously-skip-permissions` or `codex` alone.
    pub fn describe(&self, agent: Agent) -> String {
        let text = self.extra_text(agent);
        if text.is_empty() {
            agent.to_string()
        } else {
            format!("{agent} {text}")
        }
    }

    /// Config file location: `$XDG_CONFIG_HOME/agent-manager/config.json`,
    /// else `~/.config/agent-manager/config.json`.
    pub fn config_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .ok()
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("agent-manager").join("config.json")
    }

    /// Load from `path`; missing, unreadable, or invalid files degrade
    /// to defaults (documented degraded mode).
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Save to `path`, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }
}

/// Split `raw` into argv words: whitespace-separated, with single quotes,
/// double quotes, and backslash escapes. POSIX-like: backslash is literal
/// inside single quotes, escapes the next char elsewhere. Returns an
/// error naming the unbalanced quote instead of guessing.
pub fn split_shell_words(raw: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    match chars.next() {
                        Some(next) => cur.push(next),
                        None => cur.push('\\'),
                    }
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c.is_whitespace() {
                    if in_word {
                        out.push(std::mem::take(&mut cur));
                        in_word = false;
                    }
                } else if c == '\'' || c == '"' {
                    quote = Some(c);
                    in_word = true;
                } else if c == '\\' {
                    match chars.next() {
                        Some(next) => {
                            cur.push(next);
                            in_word = true;
                        }
                        None => {
                            cur.push('\\');
                            in_word = true;
                        }
                    }
                } else {
                    cur.push(c);
                    in_word = true;
                }
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote in flags".to_string());
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// Join argv words into editable text, double-quoting words containing
/// whitespace or quotes so [`split_shell_words`] round-trips them.
pub fn join_shell_words(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty()
                || a.chars()
                    .any(|c| c.is_whitespace() || c == '"' || c == '\'')
            {
                format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-manager-agents-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn every_agent_launches_its_own_binary_with_no_flags_by_default() {
        let cfg = AgentConfig::default();
        assert_eq!(cfg.active_agent, Agent::Muse);
        for agent in Agent::all() {
            let (program, args) = cfg.launch_command(*agent);
            assert_eq!(program, agent.as_str());
            assert!(args.is_empty(), "{agent} must start flag-free by default");
        }
    }

    #[test]
    fn extra_flags_append_per_agent_without_cross_talk() {
        let mut cfg = AgentConfig::default();
        cfg.set_extra_raw(Agent::Muse, "--yolo").unwrap();
        cfg.set_extra_raw(Agent::Claude, "--dangerously-skip-permissions")
            .unwrap();
        cfg.set_extra_raw(Agent::Codex, "--sandbox workspace-write")
            .unwrap();
        assert_eq!(
            cfg.launch_command(Agent::Muse),
            ("muse".to_string(), vec!["--yolo".to_string()])
        );
        assert_eq!(
            cfg.launch_command(Agent::Claude),
            (
                "claude".to_string(),
                vec!["--dangerously-skip-permissions".to_string()]
            )
        );
        assert_eq!(
            cfg.launch_command(Agent::Codex),
            (
                "codex".to_string(),
                vec!["--sandbox".to_string(), "workspace-write".to_string()]
            )
        );
        // Untouched agents stay bare.
        assert_eq!(
            cfg.launch_command(Agent::Opencode),
            ("opencode".to_string(), Vec::new())
        );
    }

    #[test]
    fn clearing_flags_restores_the_bare_command_and_drops_the_key() {
        let mut cfg = AgentConfig::default();
        cfg.set_extra_raw(Agent::Muse, "--yolo").unwrap();
        assert_eq!(cfg.launch_command(Agent::Muse).1, vec!["--yolo"]);
        cfg.set_extra_raw(Agent::Muse, "   ").unwrap();
        assert_eq!(
            cfg.launch_command(Agent::Muse),
            ("muse".to_string(), Vec::new())
        );
        assert!(!cfg.extra_args.contains_key("muse"));
    }

    #[test]
    fn invalid_flags_leave_stored_flags_untouched() {
        let mut cfg = AgentConfig::default();
        cfg.set_extra_raw(Agent::Claude, "--dangerously-skip-permissions")
            .unwrap();
        assert!(cfg.set_extra_raw(Agent::Claude, "--flag \"oops").is_err());
        assert_eq!(
            cfg.launch_command(Agent::Claude).1,
            vec!["--dangerously-skip-permissions"]
        );
    }

    #[test]
    fn splitter_handles_quotes_escapes_and_blank_input() {
        assert_eq!(split_shell_words("").unwrap(), Vec::<String>::new());
        assert_eq!(
            split_shell_words("--yolo  --verbose").unwrap(),
            vec!["--yolo", "--verbose"]
        );
        assert_eq!(
            split_shell_words("--model \"opus 4\" --flag 'a b'").unwrap(),
            vec!["--model", "opus 4", "--flag", "a b"]
        );
        assert_eq!(
            split_shell_words("--dir\\ with\\ spaces").unwrap(),
            vec!["--dir with spaces"]
        );
        assert!(split_shell_words("--flag \"oops").is_err());
        assert!(split_shell_words("--flag 'oops").is_err());
    }

    #[test]
    fn join_quotes_only_when_needed_and_round_trips() {
        assert_eq!(join_shell_words(&[]), "");
        assert_eq!(join_shell_words(&["--yolo".to_string()]), "--yolo");
        let args = vec!["--model".to_string(), "opus 4".to_string()];
        assert_eq!(join_shell_words(&args), "--model \"opus 4\"");
        assert_eq!(split_shell_words(&join_shell_words(&args)).unwrap(), args);
    }

    #[test]
    fn agent_ids_parse_case_insensitively_and_reject_unknown() {
        assert_eq!(Agent::parse("muse"), Some(Agent::Muse));
        assert_eq!(Agent::parse("  Claude "), Some(Agent::Claude));
        assert_eq!(Agent::parse("CODEX"), Some(Agent::Codex));
        assert_eq!(Agent::parse("opencode"), Some(Agent::Opencode));
        assert_eq!(Agent::parse("gpt"), None);
        assert_eq!(Agent::parse(""), None);
    }

    #[test]
    fn next_cycles_all_agents_and_wraps() {
        assert_eq!(Agent::Muse.next(), Agent::Claude);
        assert_eq!(Agent::Opencode.next(), Agent::Muse);
        let mut seen = vec![Agent::Muse];
        while *seen.last().unwrap() != Agent::Opencode {
            seen.push(seen.last().unwrap().next());
        }
        assert_eq!(seen, Agent::all());
    }

    #[test]
    fn config_round_trips_through_json_per_agent() {
        let path = tmp_path("roundtrip").join("config.json");
        let mut cfg = AgentConfig {
            active_agent: Agent::Claude,
            ..Default::default()
        };
        cfg.set_extra_raw(Agent::Muse, "--yolo").unwrap();
        cfg.set_extra_raw(Agent::Claude, "--dangerously-skip-permissions")
            .unwrap();
        cfg.save_to(&path).unwrap();
        let back = AgentConfig::load_from(&path);
        assert_eq!(back, cfg);
        assert_eq!(
            back.launch_command(Agent::Claude).1,
            vec!["--dangerously-skip-permissions"]
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn missing_or_invalid_config_degrades_to_defaults() {
        assert_eq!(
            AgentConfig::load_from(&tmp_path("missing").join("config.json")),
            AgentConfig::default()
        );
        let dir = tmp_path("invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(AgentConfig::load_from(&path), AgentConfig::default());
        // Unknown active agent also degrades instead of failing the load.
        std::fs::write(&path, r#"{"active_agent": "gpt"}"#).unwrap();
        assert_eq!(AgentConfig::load_from(&path), AgentConfig::default());
        // ... while keeping any stored flags (only the agent falls back).
        std::fs::write(
            &path,
            r#"{"active_agent": "gpt", "extra_args": {"muse": ["--yolo"]}}"#,
        )
        .unwrap();
        let kept = AgentConfig::load_from(&path);
        assert_eq!(kept.active_agent, Agent::Muse);
        assert_eq!(kept.launch_command(Agent::Muse).1, vec!["--yolo"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
