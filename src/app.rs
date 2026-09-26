//! Agent models, conversation status, and sort order.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::embedded::SpawnKind;
use crate::transcript::TranscriptMessage;

/// Conversation attention state, ordered by urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    /// Needs user attention (e.g. approval prompt, error, recent question).
    Attention,
    /// Waiting / no recent activity.
    Idle,
    /// Actively producing output.
    Working,
}

/// Max parsed links retained per list per run (PRs and related alike).
/// Cap-not-drop display keeps the full story bounded: feeding 10k links
/// keeps memory flat and surfaces [`ChatSession::links_truncated`].
pub const MAX_STORED_LINKS: usize = 50;

/// Max parsed-link rows shown per run in the sessions panel; the rest
/// fold behind an `N more` disclosure ([`visible_links`]).
pub const MAX_VISIBLE_LINKS: usize = 20;

/// One chat/agent conversation surfaced by a [`crate::providers::Provider`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub project: String,
    pub status: Status,
    /// Unix seconds of last observed activity (for display only).
    pub last_active: i64,
    /// GitHub PR URLs extracted from the conversation transcript.
    pub pr_links: Vec<String>,
    /// Non-PR references (issues, commits, file refs), same treatment.
    #[serde(default)]
    pub related_links: Vec<String>,
    /// True once either link list hit [`MAX_STORED_LINKS`].
    #[serde(default)]
    pub links_truncated: bool,
    /// Most recent chat messages parsed from the `session.jsonl` tail.
    #[serde(default)]
    pub transcript: Vec<TranscriptMessage>,
    /// True when older messages were dropped (tail or message cap).
    #[serde(default)]
    pub transcript_truncated: bool,
    /// True once a submitted prompt replaced the placeholder animal title.
    #[serde(default)]
    pub title_locked: bool,
    /// Current input line being typed into this run (Terminal focus).
    /// Session state, not PTY state: title tracking works before the
    /// child spawns and after it exits. Cleared on submit and restart.
    #[serde(default)]
    pub pending_input: String,
    /// Provider-side conversation id for `--resume` (empty for live/new
    /// runs). Survives save/load so restarts resume the same transcript.
    #[serde(default)]
    pub provider_session_id: Option<String>,
}

impl ChatSession {
    /// Merge freshly scanned links in first-seen order, capping each list
    /// at [`MAX_STORED_LINKS`] and raising `links_truncated` on overflow.
    /// Underlying data is never dropped by the display cap: the panel
    /// folds extras behind `N more` via [`visible_links`].
    pub fn push_links(&mut self, pr_fresh: Vec<String>, related_fresh: Vec<String>) {
        for link in pr_fresh {
            if !self.pr_links.contains(&link) {
                if self.pr_links.len() >= MAX_STORED_LINKS {
                    self.links_truncated = true;
                } else {
                    self.pr_links.push(link);
                }
            }
        }
        for link in related_fresh {
            if !self.related_links.contains(&link) {
                if self.related_links.len() >= MAX_STORED_LINKS {
                    self.links_truncated = true;
                } else {
                    self.related_links.push(link);
                }
            }
        }
    }
}

/// Split a retained link list into the rows the panel shows plus the
/// folded count: the first [`MAX_VISIBLE_LINKS`] stay visible, the rest
/// collapse into the `N more` disclosure.
pub fn visible_links(links: &[String]) -> (&[String], usize) {
    if links.len() > MAX_VISIBLE_LINKS {
        (&links[..MAX_VISIBLE_LINKS], links.len() - MAX_VISIBLE_LINKS)
    } else {
        (links, 0)
    }
}

/// Placeholder names for runs before the user types their first prompt.
const ANIMALS: &[&str] = &[
    "otter", "fox", "badger", "heron", "mole", "wren", "stoat", "newt", "vole", "ibex", "gecko",
    "quail", "shrew", "egret", "marmot", "dormouse", "grebe", "polecat", "siskin", "tenrec",
    "uakari", "xerus", "yak", "zapus",
];

/// Deterministic placeholder title for the n-th run (1-based).
pub fn animal_name(n: usize) -> String {
    let base = ANIMALS[(n.saturating_sub(1)) % ANIMALS.len()];
    if n > ANIMALS.len() {
        format!("{base}-{n}")
    } else {
        base.to_string()
    }
}

/// Screen-text markers suggesting `muse` waits on the user (approval
/// prompts, permission questions, errors). Matched case-insensitively.
/// This is the single reconciled marker list, precompiled once: the live
/// shell and the historic provider classifier both funnel through
/// [`classify`], so a run can never show a different status live vs
/// historic by construction, and no per-tick allocation happens here.
static ATTENTION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)approval|approve|permission|needs_input|needs input|\"error\"|\(y/n\)|allow once|allow always|would you like|press enter to confirm"#,
    )
    .expect("static attention regex")
});

/// True when `text` carries an attention marker. Allocation-free:
/// a single precompiled case-insensitive scan, no lowercase copy.
pub fn needs_attention(text: &str) -> bool {
    ATTENTION_RE.is_match(text)
}

/// A run counts as actively working while it produced output recently.
pub const WORKING_WINDOW_SECS: u64 = 60;

/// The one status classifier, owned by `app`. `screen_text` is the live
/// screen (or the historic transcript tail), `output_age` is how long ago
/// the run last produced output (`None` = never/unknown), and `exited`
/// reports the child state (always `false` for historic sessions).
/// Attention markers win over everything; an exited run without markers is
/// idle; otherwise recency inside the working window decides.
pub fn classify(
    screen_text: &str,
    output_age: Option<std::time::Duration>,
    exited: bool,
) -> Status {
    classify_with_attention(needs_attention(screen_text), output_age, exited)
}

/// [`classify`] with the marker scan already done. The shell caches the
/// scan per run and skips re-scanning unchanged screens; attention still
/// outranks exit and recency exactly as in [`classify`].
pub fn classify_with_attention(
    attention: bool,
    output_age: Option<std::time::Duration>,
    exited: bool,
) -> Status {
    if attention {
        return Status::Attention;
    }
    if exited {
        return Status::Idle;
    }
    match output_age {
        Some(age) if age < std::time::Duration::from_secs(WORKING_WINDOW_SECS) => Status::Working,
        _ => Status::Idle,
    }
}

/// Top-down sort: attention first, then idle, then working; stable by
/// `last_active` descending within a status bucket.
pub fn sort_sessions(sessions: &mut [ChatSession]) {
    sessions.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| b.last_active.cmp(&a.last_active))
    });
}

/// Left-panel section header for a status bucket.
pub fn section_title(status: Status) -> &'static str {
    match status {
        Status::Attention => "Needs input",
        Status::Idle => "Idle",
        Status::Working => "Active",
    }
}

/// Group already-sorted sessions into status sections, in [`Status`] order
/// (attention, idle, working). Returns `(status, indices)` pairs, skipping
/// empty buckets; within a bucket the slice order is preserved.
pub fn status_sections(sessions: &[ChatSession]) -> Vec<(Status, Vec<usize>)> {
    let mut out: Vec<(Status, Vec<usize>)> = Vec::new();
    for status in [Status::Attention, Status::Idle, Status::Working] {
        let ids: Vec<usize> = sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.status == status)
            .map(|(i, _)| i)
            .collect();
        if !ids.is_empty() {
            out.push((status, ids));
        }
    }
    out
}

/// Basename of the current working directory, for labeling user runs.
fn current_dir_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "muse".to_string())
}

/// Current Unix time in seconds, for run bookkeeping.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Which pane owns the keyboard: list navigation or the embedded `muse`
/// terminal. In [`Focus::Terminal`] every key (except the focus key itself)
/// is forwarded to `muse`; app navigation is suspended until focus returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// List navigation: j/k move, PgUp/PgDn page, o focuses a parsed
    /// link, Enter copies the focused link, n starts a new session,
    /// ? toggles the in-app help panel.
    #[default]
    Nav,
    /// Typing: keys go to the embedded `muse` PTY.
    Terminal,
}

/// Application state: the live run list plus list selection, keyboard
/// focus, the pending PTY spawn the main loop must start, and a counter for
/// user-created run ids. The list starts empty: entries appear only when the
/// user starts a new `muse` session in-app.
pub struct App {
    pub sessions: Vec<ChatSession>,
    pub selected: usize,
    pub focus: Focus,
    pending_spawn: Option<SpawnKind>,
    next_run: usize,
    status_msg: Option<(String, std::time::Instant)>,
    sticky_error: Option<String>,
    /// User configuration (issue #33: per-agent extra CLI flags).
    config: Config,
}

/// How long a transient status-bar message stays visible.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(3);

/// Rows moved by one PgUp/PgDn step. List state carries no viewport
/// height, so paging is a fixed step, clamped at the ends.
pub const PAGE_STEP: usize = 5;

impl App {
    pub fn new(mut sessions: Vec<ChatSession>) -> Self {
        sort_sessions(&mut sessions);
        Self {
            sessions,
            selected: 0,
            focus: Focus::Nav,
            pending_spawn: None,
            next_run: 0,
            status_msg: None,
            sticky_error: None,
            config: Config::default(),
        }
    }

    /// Install the user configuration (loaded once at startup in
    /// `main`). Tests keep the default (no extra flags).
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Spawn command for `kind` with the configured per-agent extra flags
    /// appended (issue #33). The key is the program name, so every
    /// supported agent (`muse`, `claude`, …) can carry its own flags.
    pub fn spawn_command_for(&self, kind: &SpawnKind) -> (String, Vec<String>) {
        let (program, mut args) = kind.command();
        args.extend(self.config.extra_args_for(&program));
        (program, args)
    }

    /// One-line spawn description for UI affordances (`muse --yolo`).
    pub fn spawn_command_string(&self) -> String {
        let (program, args) = self.spawn_command_for(&SpawnKind::New);
        if args.is_empty() {
            program
        } else {
            format!("{program} {}", args.join(" "))
        }
    }

    /// Flash a transient message in the status bar.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), std::time::Instant::now()));
    }

    /// Record a sticky error (spawn/PTY-write failure). It stays until an
    /// explicit [`App::clear_error`] or the next success — repaints and the
    /// transient TTL never clear it.
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.sticky_error = Some(msg.into());
    }

    /// Dismiss the sticky error, if any.
    pub fn clear_error(&mut self) {
        self.sticky_error = None;
    }

    /// The sticky error, if one is being shown.
    pub fn error_text(&self) -> Option<&str> {
        self.sticky_error.as_deref()
    }

    /// Current status-bar message. Split policy: info flashes for
    /// [`STATUS_TTL`], errors stay until dismissed or superseded by a
    /// success. A sticky error always wins over transient info.
    pub fn status_text(&self) -> Option<&str> {
        if let Some(err) = self.sticky_error.as_deref() {
            return Some(err);
        }
        self.status_msg.as_ref().and_then(|(msg, at)| {
            if at.elapsed() < STATUS_TTL {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    /// Take the pending spawn request, if any (the main loop spawns it,
    /// then owns the live handle until the next request).
    pub fn take_pending_spawn(&mut self) -> Option<SpawnKind> {
        self.pending_spawn.take()
    }

    /// Re-queue a spawn for the selected run (Retry after a spawn failure).
    /// Creates no new run entry: the failed run keeps its id and title.
    /// `kind` preserves the origin: historic/retry selections resume,
    /// live ones relaunch fresh. No-op when no run is selected.
    pub fn retry_spawn(&mut self, kind: SpawnKind) {
        if self.selected_session().is_some() {
            self.pending_spawn = Some(kind);
        }
    }

    /// Whether a spawn is queued and not yet consumed by the main loop.
    pub fn has_pending_spawn(&self) -> bool {
        self.pending_spawn.is_some()
    }

    /// Spawn kind for a sidebar selection: historic entries resume,
    /// live entries relaunch fresh.
    pub fn respawn_kind(&self, run_id: &str) -> SpawnKind {
        match self.sessions.iter().find(|s| s.id == run_id) {
            Some(s) if s.provider_session_id.is_some() => SpawnKind::Resume {
                session_id: s.provider_session_id.clone().unwrap_or_default(),
            },
            _ => SpawnKind::New,
        }
    }

    /// Remove the run entry with `run_id` (per-run close/kill: the shell
    /// drops the live PTY alongside, so `Drop` reaps the child).
    /// Returns the removed title for the confirmation flash. Selection
    /// clamps into the shrunken list; a pending spawn for the closed run
    /// is dropped with it. No-op (returns `None`) when missing.
    pub fn remove_session(&mut self, run_id: &str) -> Option<String> {
        let pos = self.sessions.iter().position(|s| s.id == run_id)?;
        // A queued spawn always belongs to the selected (newest) run; it
        // dies with that run, never with a bystander.
        let closing_selected = pos == self.selected;
        let removed = self.sessions.remove(pos);
        if self.selected >= self.sessions.len() {
            self.selected = self.sessions.len().saturating_sub(1);
        }
        if closing_selected {
            self.pending_spawn = None;
        }
        Some(removed.title)
    }

    /// True when quitting deserves a confirmation step: any run is
    /// Working or Attention. (The shell ORs in live PTYs it owns.)
    pub fn needs_quit_confirm(&self) -> bool {
        self.sessions
            .iter()
            .any(|s| matches!(s.status, Status::Working | Status::Attention))
    }

    /// Enter terminal focus: keys go to the embedded `muse`.
    pub fn focus_terminal(&mut self) {
        self.focus = Focus::Terminal;
    }

    /// Return to list navigation.
    pub fn focus_nav(&mut self) {
        self.focus = Focus::Nav;
    }

    /// Toggle between navigation and terminal focus (bound to Tab).
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Nav => Focus::Terminal,
            Focus::Terminal => Focus::Nav,
        };
    }

    pub fn is_terminal_focused(&self) -> bool {
        self.focus == Focus::Terminal
    }

    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.sessions.len();
    }

    pub fn select_prev(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.sessions.len() - 1);
    }

    /// Page down: move selection toward the tail, clamped at the last run.
    pub fn select_page_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + PAGE_STEP).min(self.sessions.len() - 1);
    }

    /// Page up: move selection toward the head, clamped at the first run.
    pub fn select_page_prev(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(PAGE_STEP);
    }

    /// Create a new live-run entry, queue a brand-new `muse` session for it,
    /// and hand it the keyboard. Switching runs never kills the others: the
    /// main loop keeps one live PTY per entry. The entry starts with a
    /// placeholder animal title until the first submitted prompt renames it.
    pub fn start_new_session(&mut self) {
        self.next_run += 1;
        let n = self.next_run;
        self.sessions.push(ChatSession {
            id: format!("run-{n}"),
            title: animal_name(n),
            project: current_dir_name(),
            status: Status::Working,
            last_active: now_secs(),
            pr_links: vec![],
            related_links: vec![],
            links_truncated: false,
            transcript: vec![],
            transcript_truncated: false,
            provider_session_id: None,
            title_locked: false,
            pending_input: String::new(),
        });
        self.selected = self.sessions.len() - 1;
        self.pending_spawn = Some(SpawnKind::New);
        self.focus = Focus::Terminal;
    }

    /// Record a prompt line the user submitted to `run_id`: the first one
    /// becomes the run's summary title so the list stays distinguishable,
    /// flashing a `renamed to '<title>'` confirmation so the silent rename
    /// is noticed. Later prompts and blank lines never rename nor flash.
    pub fn note_submitted_prompt(&mut self, run_id: &str, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let renamed = if let Some(s) = self.sessions.iter_mut().find(|s| s.id == run_id) {
            if !s.title_locked {
                s.title = crate::transcript::single_line(line);
                s.title_locked = true;
                Some(s.title.clone())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(title) = renamed {
            self.set_status(format!("renamed to '{title}'"));
        }
    }

    /// Re-sort by activity (attention, idle, working) while keeping the
    /// selection pinned to the same run.
    pub fn resort_keep_selection(&mut self) {
        let selected_id = self.selected_session().map(|s| s.id.clone());
        sort_sessions(&mut self.sessions);
        if let Some(id) = selected_id {
            if let Some(pos) = self.sessions.iter().position(|s| s.id == id) {
                self.selected = pos;
            }
        }
    }

    pub fn selected_session(&self) -> Option<&ChatSession> {
        self.sessions.get(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(id: &str, status: Status, last_active: i64) -> ChatSession {
        ChatSession {
            id: id.into(),
            title: id.into(),
            project: "proj".into(),
            status,
            last_active,
            pr_links: vec![],
            related_links: vec![],
            links_truncated: false,
            transcript: vec![],
            transcript_truncated: false,
            provider_session_id: None,
            title_locked: true,
            pending_input: String::new(),
        }
    }

    #[test]
    fn link_storage_caps_at_50_with_truncation_flag() {
        let mut s = sess("a", Status::Working, 1);
        // 60 fresh PR links: first 50 retained in order, flag raised.
        let fresh: Vec<String> = (0..60)
            .map(|n| format!("https://github.com/acme/app/pull/{n}"))
            .collect();
        s.push_links(fresh, vec![]);
        assert_eq!(s.pr_links.len(), MAX_STORED_LINKS);
        assert_eq!(s.pr_links[0], "https://github.com/acme/app/pull/0");
        assert!(s.links_truncated);
        // Related lists cap independently; duplicates never double-count.
        let related: Vec<String> = (0..55).map(|n| format!("src/f{n}.rs:1")).collect();
        s.push_links(vec![s.pr_links[0].clone()], related);
        assert_eq!(s.pr_links.len(), MAX_STORED_LINKS);
        assert_eq!(s.related_links.len(), MAX_STORED_LINKS);
    }

    #[test]
    fn visible_links_folds_beyond_20_behind_n_more() {
        let links: Vec<String> = (0..25).map(|n| format!("l{n}")).collect();
        let (shown, hidden) = visible_links(&links);
        assert_eq!(shown.len(), MAX_VISIBLE_LINKS);
        assert_eq!(hidden, 5);
        let short: Vec<String> = (0..3).map(|n| format!("l{n}")).collect();
        let (shown, hidden) = visible_links(&short);
        assert_eq!(shown.len(), 3);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn attention_sorts_first_then_idle_then_working() {
        let mut v = vec![
            sess("work", Status::Working, 99),
            sess("idle", Status::Idle, 1),
            sess("attn", Status::Attention, 1),
        ];
        sort_sessions(&mut v);
        assert_eq!(v[0].id, "attn");
        assert_eq!(v[1].id, "idle");
        assert_eq!(v[2].id, "work");
    }

    #[test]
    fn recency_breaks_status_ties() {
        let mut v = vec![
            sess("old", Status::Working, 1),
            sess("new", Status::Working, 5),
        ];
        sort_sessions(&mut v);
        assert_eq!(v[0].id, "new");
    }

    #[test]
    fn selection_wraps() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1), sess("b", Status::Idle, 2)]);
        app.select_prev();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn page_selection_moves_by_page_step_and_clamps_at_the_ends() {
        let sessions: Vec<ChatSession> = (0..8)
            .map(|n| sess(&format!("r{n}"), Status::Idle, n))
            .collect();
        let mut app = App::new(sessions);
        // Sorted newest-first; pin to the head for deterministic steps.
        app.selected = 0;
        app.select_page_next();
        assert_eq!(app.selected, PAGE_STEP);
        app.select_page_next();
        assert_eq!(app.selected, 7);
        app.select_page_next();
        assert_eq!(app.selected, 7);
        app.select_page_prev();
        assert_eq!(app.selected, 7 - PAGE_STEP);
        app.selected = 1;
        app.select_page_prev();
        assert_eq!(app.selected, 0);
        // Shorter than one step: a single page lands on the last run.
        let short: Vec<ChatSession> = (0..3)
            .map(|n| sess(&format!("s{n}"), Status::Idle, n))
            .collect();
        let mut short_app = App::new(short);
        short_app.selected = 0;
        short_app.select_page_next();
        assert_eq!(short_app.selected, 2);
        // Empty list: paging is inert.
        let mut empty = App::new(vec![]);
        empty.select_page_next();
        empty.select_page_prev();
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn startup_spawns_nothing_list_starts_empty() {
        let mut app = App::new(vec![]);
        assert!(app.sessions.is_empty());
        assert!(app.take_pending_spawn().is_none());
    }

    #[test]
    fn selection_never_spawns_switching_keeps_runs_alive() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1), sess("b", Status::Idle, 2)]);
        app.select_next();
        assert!(app.take_pending_spawn().is_none());
        app.select_prev();
        assert!(app.take_pending_spawn().is_none());
    }

    #[test]
    fn start_new_session_creates_run_and_focuses_terminal() {
        let mut app = App::new(vec![]);
        assert!(!app.is_terminal_focused());
        app.start_new_session();
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].id, "run-1");
        assert_eq!(app.selected, 0);
        assert_eq!(app.take_pending_spawn(), Some(SpawnKind::New));
        assert!(app.is_terminal_focused());
        app.focus_nav();
        app.start_new_session();
        assert_eq!(app.sessions.len(), 2);
        assert_eq!(app.sessions[1].id, "run-2");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn configured_extra_args_append_to_spawn_command() {
        // Issue #33: default spawns stay plain; configured flags append.
        use crate::config::{AgentConfig, Config};
        let mut app = App::new(vec![]);
        assert_eq!(app.spawn_command_string(), "muse");
        let (program, args) = app.spawn_command_for(&SpawnKind::New);
        assert_eq!((program.as_str(), args.len()), ("muse", 0));
        let mut cfg = Config::default();
        cfg.agents.insert(
            "muse".to_string(),
            AgentConfig {
                extra_args: vec!["--yolo".to_string()],
            },
        );
        app.set_config(cfg);
        assert_eq!(app.spawn_command_string(), "muse --yolo");
        let (program, args) = app.spawn_command_for(&SpawnKind::New);
        assert_eq!(program, "muse");
        assert_eq!(args, vec!["--yolo".to_string()]);
    }

    #[test]
    fn new_runs_get_animal_titles_until_first_prompt() {
        assert_eq!(animal_name(1), "otter");
        assert_eq!(animal_name(2), "fox");
        let mut app = App::new(vec![]);
        app.start_new_session();
        assert_eq!(app.sessions[0].title, "otter");
        app.note_submitted_prompt("run-1", "  fix the login redirect  ");
        assert_eq!(app.sessions[0].title, "fix the login redirect");
        // Second prompt does not rename: the first summary sticks.
        app.note_submitted_prompt("run-1", "something else entirely");
        assert_eq!(app.sessions[0].title, "fix the login redirect");
        // Blank lines never rename.
        let mut app2 = App::new(vec![]);
        app2.start_new_session();
        app2.note_submitted_prompt("run-1", "   ");
        assert_eq!(app2.sessions[0].title, "otter");
    }

    #[test]
    fn unified_classifier_covers_both_live_and_historic_paths() {
        use std::time::Duration;
        // Attention wins over exit and recency on either path.
        assert_eq!(
            classify(
                "Waiting for your approval",
                Some(Duration::from_secs(0)),
                false
            ),
            Status::Attention
        );
        assert_eq!(classify("allow once? (y/n)", None, true), Status::Attention);
        // Exited without markers is idle, however recent.
        assert_eq!(
            classify("done", Some(Duration::from_secs(0)), true),
            Status::Idle
        );
        // Recency decides the rest.
        assert_eq!(
            classify("working…", Some(Duration::from_secs(5)), false),
            Status::Working
        );
        assert_eq!(
            classify("old output", Some(Duration::from_secs(3600)), false),
            Status::Idle
        );
        assert_eq!(classify("nothing yet", None, false), Status::Idle);
    }

    #[test]
    fn cached_attention_agrees_with_full_scan() {
        use std::time::Duration;
        // The skip path (precomputed bit) decides exactly like the scan.
        assert_eq!(
            classify_with_attention(true, None, true),
            classify("allow once? (y/n)", None, true)
        );
        assert_eq!(
            classify_with_attention(false, Some(Duration::from_secs(5)), false),
            classify("plain output", Some(Duration::from_secs(5)), false)
        );
    }

    #[test]
    fn first_prompt_rename_flashes_confirmation() {
        let mut app = App::new(vec![]);
        app.start_new_session();
        assert_eq!(app.status_text(), None);
        app.note_submitted_prompt("run-1", "  fix the login redirect  ");
        assert_eq!(app.sessions[0].title, "fix the login redirect");
        assert_eq!(
            app.status_text(),
            Some("renamed to 'fix the login redirect'")
        );
        // Second prompt keeps the first title and leaves the flash alone.
        app.note_submitted_prompt("run-1", "something else entirely");
        assert_eq!(app.sessions[0].title, "fix the login redirect");
        assert_eq!(
            app.status_text(),
            Some("renamed to 'fix the login redirect'")
        );
        // Blank lines and unknown ids flash nothing.
        let mut app2 = App::new(vec![]);
        app2.start_new_session();
        app2.note_submitted_prompt("run-1", "   ");
        assert_eq!(app2.status_text(), None);
        app2.note_submitted_prompt("run-9", "hello");
        assert_eq!(app2.status_text(), None);
    }

    #[test]
    fn attention_markers_match_approval_and_error_text() {
        assert!(needs_attention("Waiting for your approval to proceed"));
        assert!(needs_attention("Allow once? (y/n)"));
        assert!(needs_attention("permission denied by policy"));
        assert!(needs_attention("Tool failed with \"error\""));
        // Precompiled case-insensitive scan: same hits as the old
        // lowercase-copy version, without the per-tick allocation.
        assert!(needs_attention(
            "APPROVAL REQUIRED — Press ENTER to confirm"
        ));
        assert!(needs_attention("Would You Like to continue?"));
        assert!(!needs_attention("Muse Code 1.4.0"));
        assert!(!needs_attention(""));
    }

    #[test]
    fn resort_keeps_selection_on_the_same_run() {
        let mut app = App::new(vec![
            sess("a", Status::Working, 5),
            sess("b", Status::Idle, 1),
        ]);
        // Constructor sorts idle-first: [b, a]; select "a".
        assert_eq!(app.sessions[1].id, "a");
        app.selected = 1;
        app.sessions[0].status = Status::Attention; // "b" jumps first
        app.resort_keep_selection();
        assert_eq!(app.sessions[app.selected].id, "a");
        assert_eq!(app.sessions[0].id, "b");
    }

    #[test]
    fn sections_group_by_status_in_sort_order_skipping_empty() {
        let v = vec![
            sess("w1", Status::Working, 9),
            sess("i1", Status::Idle, 1),
            sess("a1", Status::Attention, 1),
            sess("w2", Status::Working, 2),
        ];
        // NB: not sorted; sections still bucket in Attention/Idle/Working
        // order and preserve slice order within a bucket.
        let sections = status_sections(&v);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].0, Status::Attention);
        assert_eq!(sections[0].1, vec![2]);
        assert_eq!(sections[1].0, Status::Idle);
        assert_eq!(sections[1].1, vec![1]);
        assert_eq!(sections[2].0, Status::Working);
        assert_eq!(sections[2].1, vec![0, 3]);
        assert_eq!(section_title(Status::Attention), "Needs input");
        assert_eq!(section_title(Status::Idle), "Idle");
        assert_eq!(section_title(Status::Working), "Active");
        // Empty buckets are skipped.
        let only_idle = vec![sess("i", Status::Idle, 1)];
        let sections = status_sections(&only_idle);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, Status::Idle);
    }

    #[test]
    fn sticky_error_wins_over_transient_until_dismissed() {
        let mut app = App::new(vec![]);
        assert_eq!(app.error_text(), None);
        app.set_status("copied selection (12 chars)");
        assert_eq!(app.status_text(), Some("copied selection (12 chars)"));
        // An error supersedes info and survives: later info flashes do not
        // replace it, and repaints (repeated reads) never clear it.
        app.set_error("pty write failed");
        assert_eq!(app.error_text(), Some("pty write failed"));
        app.set_status("pasted 3 chars");
        assert_eq!(app.status_text(), Some("pty write failed"));
        assert_eq!(app.status_text(), Some("pty write failed"));
        // Explicit dismissal falls back to whatever transient info is live.
        app.clear_error();
        assert_eq!(app.error_text(), None);
        assert_eq!(app.status_text(), Some("pasted 3 chars"));
    }

    #[test]
    fn close_removes_entry_clamps_selection_and_drops_its_spawn() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1), sess("b", Status::Idle, 2)]);
        // Constructor sorts recency-desc: [b, a]; select "a" (index 1).
        app.selected = 1;
        app.pending_spawn = Some(SpawnKind::New);
        assert_eq!(app.remove_session("a"), Some("a".to_string()));
        // Closing the selected run drops its queued spawn; selection
        // clamps back into the shrunken list.
        assert!(app.take_pending_spawn().is_none());
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.selected, 0);
        // Missing ids are a no-op; dirty runs force a quit confirm.
        assert_eq!(app.remove_session("zzz"), None);
        assert!(!app.needs_quit_confirm());
        app.sessions[0].status = Status::Working;
        assert!(app.needs_quit_confirm());
        app.sessions[0].status = Status::Attention;
        assert!(app.needs_quit_confirm());
    }

    #[test]
    fn respawn_kind_resumes_historic_and_renews_live() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1)]);
        assert_eq!(app.respawn_kind("a"), SpawnKind::New);
        assert!(!app.has_pending_spawn());
        app.sessions[0].provider_session_id = Some("s-1".into());
        assert_eq!(
            app.respawn_kind("a"),
            SpawnKind::Resume {
                session_id: "s-1".into()
            }
        );
        assert_eq!(app.respawn_kind("missing"), SpawnKind::New);
        app.retry_spawn(SpawnKind::New);
        assert!(app.has_pending_spawn());
    }

    #[test]
    fn retry_requeues_spawn_without_new_run_entry() {
        let mut app = App::new(vec![]);
        // No selection: no-op, never spawns.
        app.retry_spawn(SpawnKind::New);
        assert!(app.take_pending_spawn().is_none());
        app.start_new_session();
        assert_eq!(app.sessions.len(), 1);
        // Simulate the main loop consuming the spawn, then failing.
        assert_eq!(app.take_pending_spawn(), Some(SpawnKind::New));
        app.retry_spawn(SpawnKind::New);
        assert_eq!(app.take_pending_spawn(), Some(SpawnKind::New));
        // Retry reuses the same run id: no extra entry.
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].id, "run-1");
    }

    #[test]
    fn focus_key_toggles_between_nav_and_typing() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1)]);
        assert_eq!(app.focus, Focus::Nav);
        app.toggle_focus();
        assert!(app.is_terminal_focused());
        app.toggle_focus();
        assert_eq!(app.focus, Focus::Nav);
        app.focus_terminal();
        app.focus_nav();
        assert_eq!(app.focus, Focus::Nav);
    }
}
