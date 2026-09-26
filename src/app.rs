//! Agent models, conversation status, and sort order.

use serde::{Deserialize, Serialize};

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
    /// Most recent chat messages parsed from the `session.jsonl` tail.
    #[serde(default)]
    pub transcript: Vec<TranscriptMessage>,
    /// True when older messages were dropped (tail or message cap).
    #[serde(default)]
    pub transcript_truncated: bool,
    /// True once a submitted prompt replaced the placeholder animal title.
    #[serde(default)]
    pub title_locked: bool,
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
pub fn needs_attention(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "approval",
        "approve",
        "permission",
        "needs_input",
        "needs input",
        "\"error\"",
        "(y/n)",
        "allow once",
        "allow always",
        "would you like",
        "press enter to confirm",
    ];
    let lowered = text.to_lowercase();
    MARKERS.iter().any(|m| lowered.contains(m))
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
    /// List navigation: j/k select, PgUp/PgDn scroll, n new session.
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
}

/// How long a transient status-bar message stays visible.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(3);

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
    /// No-op when no run is selected.
    pub fn retry_spawn(&mut self) {
        if self.selected_session().is_some() {
            self.pending_spawn = Some(SpawnKind::New);
        }
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
            transcript: vec![],
            transcript_truncated: false,
            title_locked: false,
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
            transcript: vec![],
            transcript_truncated: false,
            title_locked: true,
        }
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
    fn retry_requeues_spawn_without_new_run_entry() {
        let mut app = App::new(vec![]);
        // No selection: no-op, never spawns.
        app.retry_spawn();
        assert!(app.take_pending_spawn().is_none());
        app.start_new_session();
        assert_eq!(app.sessions.len(), 1);
        // Simulate the main loop consuming the spawn, then failing.
        assert_eq!(app.take_pending_spawn(), Some(SpawnKind::New));
        app.retry_spawn();
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
