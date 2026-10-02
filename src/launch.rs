//! Two-dimensional new-session launch: working folder × agent CLI.
//!
//! Framework-free by contract (epic #60): plain structs + pure functions,
//! no toolkit types, so every native shell binds one model. The picker
//! picks a [`LaunchSelection`] (folder + CLI + resolved yolo); the spawn
//! seam turns it into `(program, args, cwd)` with the per-agent yolo flag.
//!
//! Consensus from the 10-persona review (see `docs/new-session-picker.md`):
//! split-button repeat-last (`n` / `+ New` main), full picker on `N` / `▾`,
//! yolo as a per-run tri-state defaulting safe, autodetect before listing.

use serde::{Deserialize, Serialize};

/// Agent CLIs the picker can offer, in default preference order. `muse`
/// stays first: the historic default and the README prerequisite.
pub const SUPPORTED_CLIS: &[&str] = &["muse", "claude", "opencode", "codex"];

/// Cap for the persisted most-recently-used folder list (issue #29 family:
/// bounded display over unbounded data — same rule as link caps).
pub const MAX_RECENT_FOLDERS: usize = 10;

/// Canonical yolo flag per known CLI. `None` means the CLI has no
/// picker-supported yolo flag: the toggle hides instead of guessing.
/// Values match [`crate::embedded::Harness::yolo_flag`] (verified against
/// live CLI references: `muse --yolo`, `claude
/// --dangerously-skip-permissions`, `codex
/// --dangerously-bypass-approvals-and-sandbox`, `opencode --auto`);
/// unknown ids map to `None` so future harnesses never get a guessed flag.
pub fn yolo_flag_for(cli: &str) -> Option<&'static str> {
    match cli {
        "muse" => Some("--yolo"),
        "claude" => Some("--dangerously-skip-permissions"),
        "codex" => Some("--dangerously-bypass-approvals-and-sandbox"),
        "opencode" => Some("--auto"),
        _ => None,
    }
}

/// One CLI row in the picker: always listed, never silently dropped.
/// `path` is `Some` when the binary resolves on this host (autodetected),
/// `None` when it is missing from `PATH`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableCli {
    pub id: String,
    pub program: String,
    pub path: Option<String>,
    pub available: bool,
}

impl AvailableCli {
    pub fn missing(id: &str) -> Self {
        Self {
            id: id.to_string(),
            program: id.to_string(),
            path: None,
            available: false,
        }
    }
}

/// Autodetect the supported CLIs against the process `PATH` (plus a few
/// GUI-sparse extra dirs, see [`extra_search_dirs`]). Cheap `which`-style
/// probe only — no `--version` subprocess, no background threads, no
/// cache: callers re-run it on picker open so the list is never stale.
/// Order follows [`SUPPORTED_CLIS`].
pub fn detect_available_clis() -> Vec<AvailableCli> {
    detect_with_path(&std::env::var_os("PATH").unwrap_or_default())
}

/// Pure half of [`detect_available_clis`] over an explicit `PATH` value
/// (headlessly testable on every OS).
pub fn detect_with_path(path_value: &std::ffi::OsStr) -> Vec<AvailableCli> {
    let exts = pathext_suffixes();
    let mut out = Vec::with_capacity(SUPPORTED_CLIS.len());
    for id in SUPPORTED_CLIS {
        let found = resolve_bare_name(id, path_value, &exts).or_else(|| {
            extra_search_dirs()
                .iter()
                .find_map(|d| probe_dir(d, id, &exts))
        });
        let available = found.is_some();
        out.push(AvailableCli {
            id: id.to_string(),
            program: id.to_string(),
            path: found.map(|p| p.to_string_lossy().into_owned()),
            available,
        });
    }
    out
}

/// First available CLI id, if any (repeat-last / default resolution falls
/// back to this before the hardcoded `"muse"`).
pub fn first_available(clis: &[AvailableCli]) -> Option<String> {
    clis.iter().find(|c| c.available).map(|c| c.id.clone())
}

/// Resolve the effective CLI: explicit pick wins, then the last-used CLI
/// (if still supported), then the configured default, then the first
/// autodetected binary, then `"muse"` (the historic spawn, whose failure
/// stays fail-visible via the sticky error + Retry).
pub fn resolve_effective_cli(
    explicit: Option<&str>,
    last_used: Option<&str>,
    default_cli: Option<&str>,
    catalog: &[AvailableCli],
) -> String {
    if let Some(cli) = explicit.filter(|c| !c.is_empty()) {
        return cli.to_string();
    }
    let supported = |c: &str| SUPPORTED_CLIS.contains(&c);
    if let Some(last) = last_used.filter(|c| supported(c)) {
        return last.to_string();
    }
    if let Some(def) = default_cli.filter(|c| supported(c)) {
        return def.to_string();
    }
    first_available(catalog).unwrap_or_else(|| "muse".to_string())
}

/// Per-run yolo choice in the picker: safe by default, explicit per run,
/// never auto-written back to the config (see `docs/new-session-picker.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum YoloChoice {
    /// Follow the per-agent config default (`agents.<cli>.yolo`, off unless set).
    #[default]
    UseDefault,
    /// Force yolo on for this run only.
    ForceOn,
    /// Force yolo off for this run only.
    ForceOff,
}

impl YoloChoice {
    /// Cycle for the `y` key: default → on → off → default.
    pub fn cycle(self) -> Self {
        match self {
            Self::UseDefault => Self::ForceOn,
            Self::ForceOn => Self::ForceOff,
            Self::ForceOff => Self::UseDefault,
        }
    }

    /// Short label for the picker row + status flash (text, never color-only).
    pub fn label(self) -> &'static str {
        match self {
            Self::UseDefault => "default",
            Self::ForceOn => "on once",
            Self::ForceOff => "off once",
        }
    }

    /// Resolve against the persisted per-agent default.
    pub fn resolve(self, default: bool) -> bool {
        match self {
            Self::UseDefault => default,
            Self::ForceOn => true,
            Self::ForceOff => false,
        }
    }
}

/// A confirmed 2D launch: where the child spawns, with what, how freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSelection {
    pub cli: String,
    pub cwd: Option<String>,
    pub yolo: bool,
}

impl LaunchSelection {
    pub fn new(cli: String, cwd: Option<String>, yolo: bool) -> Self {
        Self { cli, cwd, yolo }
    }

    /// One-line spawn preview for the picker + status flash
    /// (`muse --yolo in ~/api`), so the launch is verifiable before it runs.
    pub fn preview(&self) -> String {
        let mut cmd = self.cli.clone();
        if self.yolo {
            if let Some(flag) = yolo_flag_for(&self.cli) {
                cmd.push(' ');
                cmd.push_str(flag);
            }
        }
        match &self.cwd {
            Some(dir) => format!("{cmd} in {dir}"),
            None => cmd,
        }
    }
}

/// Build the spawn `(program, args)` for a resolved [`LaunchSelection`]:
/// the CLI program plus the canonical yolo flag when on (deduped against
/// a user-configured identical `extra_args` entry). Configured
/// `extra_args` ride along separately in `App::spawn_command_for` — this
/// is the yolo half only, so shells never synthesize flags themselves.
pub fn yolo_args_for(selection: &LaunchSelection) -> Vec<String> {
    if !selection.yolo {
        return Vec::new();
    }
    yolo_flag_for(&selection.cli)
        .map(|f| vec![f.to_string()])
        .unwrap_or_default()
}

/// Push `dir` to the front of the MRU folder list: deduped, capped at
/// [`MAX_RECENT_FOLDERS`], blanks ignored. Pure so shells and tests share it.
pub fn push_recent_folder(recents: &mut Vec<String>, dir: &str) {
    let dir = dir.trim();
    if dir.is_empty() {
        return;
    }
    recents.retain(|d| d != dir);
    recents.insert(0, dir.to_string());
    recents.truncate(MAX_RECENT_FOLDERS);
}

/// Keyboard-first picker model: two axes (folder × CLI) plus the yolo
/// tri-state. Framework-free: shells render it natively and forward keys.
/// Enter confirms, Esc cancels with zero side effects.
#[derive(Debug, Clone)]
pub struct LaunchPicker {
    pub clis: Vec<AvailableCli>,
    pub cli_cursor: usize,
    pub folder_input: String,
    pub recent_folders: Vec<String>,
    pub yolo: YoloChoice,
    /// Which axis owns typing: true = folder input, false = CLI/yolo
    /// axis (`c` steps CLI, `y` cycles yolo, typing does nothing).
    pub folder_focus: bool,
}

impl LaunchPicker {
    pub fn new(
        clis: Vec<AvailableCli>,
        default_cli: Option<&str>,
        default_cwd: Option<&str>,
        recent_folders: Vec<String>,
    ) -> Self {
        let effective = resolve_effective_cli(None, None, default_cli, &clis);
        let cli_cursor = clis.iter().position(|c| c.id == effective).unwrap_or(0);
        // Preselect the first recent folder when no default is set, so
        // repeat-folder is one Enter (recognition over recall).
        let folder_input = match default_cwd {
            Some(dir) => dir.to_string(),
            None => recent_folders.first().cloned().unwrap_or_default(),
        };
        Self {
            clis,
            cli_cursor,
            folder_input,
            recent_folders,
            yolo: YoloChoice::UseDefault,
            folder_focus: true,
        }
    }

    pub fn selected_cli(&self) -> &AvailableCli {
        &self.clis[self.cli_cursor % self.clis.len().max(1)]
    }

    pub fn step_cli(&mut self, forward: bool) {
        if self.clis.is_empty() {
            return;
        }
        let n = self.clis.len();
        self.cli_cursor = if forward {
            (self.cli_cursor + 1) % n
        } else {
            (self.cli_cursor + n - 1) % n
        };
    }

    /// Step the folder input through the recent-folder presets: cycling
    /// recents into the input keeps full editability (type to refine,
    /// Backspace to clear to inherit).
    pub fn step_recent(&mut self, forward: bool) {
        if self.recent_folders.is_empty() {
            return;
        }
        let current = self
            .recent_folders
            .iter()
            .position(|r| *r == self.folder_input);
        let next = match current {
            Some(i) if forward => (i + 1) % self.recent_folders.len(),
            Some(i) => (i + self.recent_folders.len() - 1) % self.recent_folders.len(),
            None if forward => 0,
            None => self.recent_folders.len() - 1,
        };
        self.folder_input = self.recent_folders[next].clone();
    }

    /// Picker key dispatch. Returns true when consumed: printable keys
    /// extend the folder input, Up/Down steps recent presets, Left/Right
    /// or `c` steps the CLI axis, Tab jumps folder ↔ CLI, `y` cycles the
    /// yolo tri-state, `ctrl`/`platform` combos never count as text.
    /// Confirm (Enter) and cancel (Esc) are owned by the shell, which
    /// validates the folder before spawning.
    pub fn key(&mut self, key: &str, key_char: Option<&str>, ctrl: bool, platform: bool) -> bool {
        match key {
            "tab" => {
                self.folder_focus = !self.folder_focus;
            }
            "up" => {
                self.step_recent(false);
            }
            "down" => {
                self.step_recent(true);
            }
            "left" => {
                self.step_cli(false);
            }
            "right" => {
                self.step_cli(true);
            }
            "backspace" => {
                if self.folder_focus {
                    self.folder_input.pop();
                }
            }
            _ => {
                if ctrl || platform {
                    return true;
                }
                if key.eq_ignore_ascii_case("c") && !self.folder_focus {
                    self.step_cli(true);
                    return true;
                }
                if key.eq_ignore_ascii_case("y") && !self.folder_focus {
                    self.yolo = self.yolo.cycle();
                    return true;
                }
                if self.folder_focus {
                    if let Some(c) = key_char {
                        if c.chars().count() == 1 {
                            self.folder_input.push_str(c);
                        }
                    }
                }
            }
        }
        true
    }

    /// Confirm into a resolved selection: blank folder means inherit
    /// (`None`, the historic behavior); `~` expands via the shared
    /// [`crate::app::expand_cwd_input`] rule so both captures agree.
    pub fn confirm(&self, yolo_default: bool) -> LaunchSelection {
        LaunchSelection {
            cli: self.selected_cli().id.clone(),
            cwd: crate::app::expand_cwd_input(&self.folder_input),
            yolo: self.yolo.resolve(yolo_default),
        }
    }

    /// Preview line for the picker footer (`runs: muse --yolo in ~/api`).
    pub fn preview(&self, yolo_default: bool) -> String {
        self.confirm(yolo_default).preview()
    }
}

// --- PATH probing (mirrors `embedded::resolve_bare_program`) ---

fn pathext_suffixes() -> Vec<String> {
    std::env::var_os("PATHEXT")
        .map(|v| {
            std::env::split_paths(&v)
                .filter_map(|e| e.to_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![".exe".to_owned()])
}

fn probe_dir(dir: &str, program: &str, exts: &[String]) -> Option<std::path::PathBuf> {
    let dir = expand_home(dir)?;
    let base = dir.join(program);
    if base.is_file() {
        return Some(base);
    }
    exts.iter().find_map(|ext| {
        let candidate = dir.join(format!("{program}{ext}"));
        candidate.is_file().then_some(candidate)
    })
}

fn expand_home(dir: &str) -> Option<std::path::PathBuf> {
    if let Some(rest) = dir.strip_prefix("~/") {
        return crate::app::home_dir().map(|h| std::path::PathBuf::from(h).join(rest));
    }
    if dir == "~" {
        return crate::app::home_dir().map(std::path::PathBuf::from);
    }
    Some(std::path::PathBuf::from(dir))
}

/// Extra dirs for GUI-launched processes whose `PATH` misses shell-added
/// entries (macOS `/opt/homebrew`, Linux `/snap/bin`, user `~/.local/bin`).
fn extra_search_dirs() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        vec![
            "/opt/homebrew/bin".to_string(),
            "/usr/local/bin".to_string(),
            "~/.local/bin".to_string(),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            "~/.local/bin".to_string(),
            "/snap/bin".to_string(),
            "/var/lib/flatpak/exports/bin".to_string(),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        Vec::new()
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        Vec::new()
    }
}

fn resolve_bare_name(
    program: &str,
    path_value: &std::ffi::OsStr,
    exts: &[String],
) -> Option<std::path::PathBuf> {
    if program.is_empty() || program.contains('/') || program.contains('\\') {
        return None;
    }
    let ext_refs: Vec<&str> = exts.iter().map(String::as_str).collect();
    for dir in std::env::split_paths(path_value) {
        let base = dir.join(program);
        if base.is_file() {
            return Some(base);
        }
        for ext in &ext_refs {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(ids: &[(&str, bool)]) -> Vec<AvailableCli> {
        ids.iter()
            .map(|(id, up)| AvailableCli {
                id: id.to_string(),
                program: id.to_string(),
                path: up.then(|| format!("/bin/{id}")),
                available: *up,
            })
            .collect()
    }

    #[test]
    fn yolo_flags_cover_documented_clis_only() {
        assert_eq!(yolo_flag_for("muse"), Some("--yolo"));
        assert_eq!(
            yolo_flag_for("claude"),
            Some("--dangerously-skip-permissions")
        );
        assert_eq!(
            yolo_flag_for("codex"),
            Some("--dangerously-bypass-approvals-and-sandbox")
        );
        assert_eq!(yolo_flag_for("opencode"), Some("--auto"));
        assert_eq!(yolo_flag_for("future"), None);
    }

    #[test]
    fn yolo_choice_cycles_and_resolves() {
        assert_eq!(YoloChoice::UseDefault.cycle(), YoloChoice::ForceOn);
        assert_eq!(YoloChoice::ForceOn.cycle(), YoloChoice::ForceOff);
        assert_eq!(YoloChoice::ForceOff.cycle(), YoloChoice::UseDefault);
        assert!(YoloChoice::ForceOn.resolve(false));
        assert!(!YoloChoice::ForceOff.resolve(true));
        assert!(YoloChoice::UseDefault.resolve(true));
        assert!(!YoloChoice::UseDefault.resolve(false));
    }

    #[test]
    fn effective_cli_prefers_explicit_then_last_then_default_then_detect() {
        let cat = catalog(&[("muse", true), ("claude", true)]);
        assert_eq!(
            resolve_effective_cli(Some("codex"), Some("claude"), Some("muse"), &cat),
            "codex"
        );
        assert_eq!(
            resolve_effective_cli(None, Some("claude"), Some("muse"), &cat),
            "claude"
        );
        assert_eq!(
            resolve_effective_cli(None, None, Some("claude"), &cat),
            "claude"
        );
        assert_eq!(resolve_effective_cli(None, None, None, &cat), "muse");
        // Unsupported last/default fall through to autodetect.
        assert_eq!(
            resolve_effective_cli(None, Some("nope"), Some("nope"), &cat),
            "muse"
        );
        // Nothing detected anywhere: historic hardcoded default keeps the
        // failure fail-visible (sticky error + Retry) instead of an empty pick.
        assert_eq!(resolve_effective_cli(None, None, None, &[]), "muse");
    }

    #[test]
    fn selection_preview_names_command_and_folder() {
        let sel = LaunchSelection::new("muse".to_string(), Some("/tmp/api".to_string()), true);
        assert_eq!(sel.preview(), "muse --yolo in /tmp/api");
        let plain = LaunchSelection::new("claude".to_string(), None, false);
        assert_eq!(plain.preview(), "claude");
        // Unknown CLI: no guessed flag, never a broken command.
        let unknown = LaunchSelection::new("future".to_string(), None, true);
        assert_eq!(unknown.preview(), "future");
        assert!(yolo_args_for(&unknown).is_empty());
        // Verified flags ride the preview + args for codex/opencode.
        let cx = LaunchSelection::new("codex".to_string(), None, true);
        assert_eq!(
            cx.preview(),
            "codex --dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(
            yolo_args_for(&cx),
            vec!["--dangerously-bypass-approvals-and-sandbox".to_string()]
        );
        let oc = LaunchSelection::new("opencode".to_string(), None, true);
        assert_eq!(oc.preview(), "opencode --auto");
    }

    #[test]
    fn recents_dedupe_cap_and_ignore_blanks() {
        let mut recents = vec!["/b".to_string()];
        push_recent_folder(&mut recents, "/a");
        assert_eq!(recents, vec!["/a", "/b"]);
        push_recent_folder(&mut recents, "/a");
        assert_eq!(recents, vec!["/a", "/b"]);
        push_recent_folder(&mut recents, "   ");
        assert_eq!(recents, vec!["/a", "/b"]);
        for i in 0..20 {
            push_recent_folder(&mut recents, &format!("/d{i}"));
        }
        assert_eq!(recents.len(), MAX_RECENT_FOLDERS);
        assert_eq!(recents[0], "/d19");
    }

    #[test]
    fn picker_confirms_blank_as_inherit_and_cycles_clis() {
        let cat = catalog(&[("muse", true), ("claude", false)]);
        let mut picker = LaunchPicker::new(cat, Some("claude"), None, vec![]);
        // Default CLI preselected even when missing (listed, not hidden).
        assert_eq!(picker.selected_cli().id, "claude");
        picker.step_cli(true);
        assert_eq!(picker.selected_cli().id, "muse");
        picker.step_cli(false);
        assert_eq!(picker.selected_cli().id, "claude");
        // Blank folder inherits; yolo resolves against the config default.
        let sel = picker.confirm(false);
        assert_eq!(sel.cwd, None);
        assert!(!sel.yolo);
        picker.yolo = YoloChoice::ForceOn;
        assert!(picker.confirm(false).yolo);
        picker.folder_input = "~/proj".to_string();
        let home = crate::app::home_dir().expect("test env has a home dir");
        assert_eq!(picker.confirm(false).cwd, Some(format!("{home}/proj")));
    }

    #[test]
    fn picker_key_drives_both_axes_and_yolo() {
        let cat = catalog(&[("muse", true), ("claude", true)]);
        let mut picker = LaunchPicker::new(cat, Some("muse"), None, vec!["/tmp".to_string()]);
        // Recent preselected when no default folder is set.
        assert_eq!(picker.folder_input, "/tmp");
        // Typing refines the folder input; Backspace edits.
        assert!(picker.key("x", Some("x"), false, false));
        assert!(picker.folder_input.ends_with('x'));
        assert!(picker.key("backspace", None, false, false));
        assert_eq!(picker.folder_input, "/tmp");
        // Ctrl combos never count as path text.
        assert!(picker.key("c", Some("c"), true, false));
        assert_eq!(picker.folder_input, "/tmp");
        // Tab jumps to the CLI axis: `c` steps it, `y` cycles yolo.
        assert!(picker.key("tab", None, false, false));
        assert!(!picker.folder_focus);
        assert!(picker.key("c", Some("c"), false, false));
        assert_eq!(picker.selected_cli().id, "claude");
        assert_eq!(picker.yolo, YoloChoice::UseDefault);
        assert!(picker.key("y", Some("y"), false, false));
        assert_eq!(picker.yolo, YoloChoice::ForceOn);
        // Typing on the CLI axis does nothing to the folder.
        assert!(picker.key("q", Some("q"), false, false));
        assert_eq!(picker.folder_input, "/tmp");
        // Left/Right step the CLI axis directly.
        assert!(picker.key("left", None, false, false));
        assert_eq!(picker.selected_cli().id, "muse");
        assert!(picker.key("right", None, false, false));
        assert_eq!(picker.selected_cli().id, "claude");
        // Up/Down cycles recents into the input.
        picker.folder_focus = true;
        assert!(picker.key("down", None, false, false));
        assert_eq!(picker.folder_input, "/tmp");
    }

    #[test]
    fn detect_lists_every_supported_cli_in_order() {
        let clis = detect_with_path(std::ffi::OsStr::new(""));
        let ids: Vec<&str> = clis.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, SUPPORTED_CLIS);
    }

    #[test]
    fn detect_finds_a_fake_cli_on_path() {
        let dir = std::env::temp_dir().join(format!(
            "agent-manager-launch-detect-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("codex"), b"fake").unwrap();
        let clis = detect_with_path(dir.as_os_str());
        let codex = clis.iter().find(|c| c.id == "codex").unwrap();
        assert!(codex.available);
        assert!(codex.path.as_deref().is_some_and(|p| p.contains("codex")));
        let muse = clis.iter().find(|c| c.id == "muse").unwrap();
        assert!(!muse.available || muse.path.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
