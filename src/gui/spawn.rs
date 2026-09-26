//! Spawn + key-forwarding logic for [`super::shell::ShellView`].
//!
//! Headless by design (plain structs, real PTYs, no window): starting the
//! queued `muse`, retrying failed spawns, and forwarding terminal-focus
//! keystrokes — the spawn/pump/key paths — are all covered without a gpui
//! harness.

use crate::embedded::EmbeddedPty;

use super::keys::{keystroke_to_pty, KeyPress};
use super::runs::Run;
use super::shell::ShellView;

impl ShellView {
    /// Start the queued `muse` for the newly created run. Returns true when
    /// a spawn was consumed: success adds a live view, failure sticks an
    /// error banner — either way the screen changed and needs a repaint.
    pub(crate) fn spawn_queued(&mut self) -> bool {
        if let Some(kind) = self.app.take_pending_spawn() {
            let run_id = self.active_id().unwrap_or_default();
            match EmbeddedPty::spawn_kind(&kind, self.cols, self.rows) {
                Ok(pty) => {
                    self.runs.insert(run_id.clone(), Run::new(pty));
                    self.note_spawn_success(&run_id);
                }
                Err(e) => {
                    self.app.set_error(e.to_string());
                }
            }
            true
        } else {
            false
        }
    }

    /// Success bookkeeping for a fresh spawn: refresh output recency and
    /// clear any sticky error (errors stay until dismissed/next success).
    pub(crate) fn note_spawn_success(&mut self, run_id: &str) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.last_output = std::time::Instant::now();
        }
        self.app.clear_error();
    }

    /// Retry is offered when a spawn actually failed and the selected run
    /// still owns no live PTY. Gating on the recorded failure (not just a
    /// missing PTY) keeps a fast-typed `r` reaching a still-starting child
    /// instead of queueing a duplicate spawn.
    pub(crate) fn can_retry(&self) -> bool {
        if self.app.error_text().is_none() {
            return false;
        }
        match self.active_id() {
            Some(id) => !self.runs.contains_key(&id),
            None => false,
        }
    }

    /// Retry action for a failed spawn: re-queue the run's spawn kind for
    /// the selected run (same id, no new entry) — `New` for live runs,
    /// `Resume` for historic entries so a failed re-attach retries the
    /// same provider session. Never touches a live PTY.
    pub(crate) fn retry_spawn(&mut self) {
        if self.can_retry() {
            if let Some(id) = self.active_id() {
                let kind = self.app.respawn_kind(&id);
                self.app.retry_spawn(kind);
            }
        }
    }

    /// Forward one gpui key event to the active PTY (Terminal focus),
    /// tracking the input line so the first submitted prompt titles the run.
    pub(crate) fn forward_key(&mut self, key: &str, key_char: Option<&str>, ctrl: bool, alt: bool) {
        let Some(id) = self.active_id() else {
            return;
        };
        // Title tracking runs even when no PTY is attached yet: the
        // pending line lives on the session, not the child.
        if let Some(s) = self.app.sessions.iter_mut().find(|s| s.id == id) {
            match key {
                "enter" => {
                    let line = std::mem::take(&mut s.pending_input);
                    self.app.note_submitted_prompt(&id, &line);
                }
                "backspace" => {
                    s.pending_input.pop();
                }
                _ => {
                    let text = key_char.filter(|c| !c.is_empty()).unwrap_or(key);
                    if text.chars().count() == 1 && !ctrl {
                        s.pending_input.push_str(text);
                    }
                }
            }
        }
        let press = KeyPress {
            key,
            key_char,
            ctrl,
            alt,
        };
        if let Some(bytes) = keystroke_to_pty(&press) {
            if let Some(run) = self.runs.get_mut(&id) {
                if let Err(e) = run.pty.write_input(&bytes) {
                    self.app.set_error(e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::nav::NavAction;
    use super::super::runs::{insert_test_pty, test_shell};

    #[test]
    fn retry_requeues_failed_spawn_and_never_touches_live_pty() {
        let mut view = test_shell();
        // No run: retry is inert.
        assert!(!view.can_retry());
        assert_eq!(view.nav_action("r", false), NavAction::None);
        view.retry_spawn();
        assert!(view.app.take_pending_spawn().is_none());

        view.app.start_new_session();
        // Fresh run, failure not yet recorded: `r` stays inert so fast
        // typing still reaches the starting child.
        assert!(!view.can_retry());
        assert_eq!(view.nav_action("r", false), NavAction::None);

        // Simulate the pump consuming the spawn and failing.
        let _ = view.app.take_pending_spawn();
        view.app
            .set_error("failed to spawn `muse`: missing binary".to_string());
        assert!(view.can_retry());
        assert_eq!(view.nav_action("r", false), NavAction::Retry);
        view.retry_spawn();
        assert_eq!(
            view.app.take_pending_spawn(),
            Some(crate::embedded::SpawnKind::New)
        );
        assert_eq!(
            view.status_text(),
            "failed to spawn `muse`: missing binary · r: retry"
        );

        // A live PTY disables retry: the run is already up.
        let _ = view.app.take_pending_spawn();
        insert_test_pty(&mut view, "sleep", &["5"]);
        assert!(!view.can_retry());
        assert_eq!(view.nav_action("r", false), NavAction::None);
    }

    #[test]
    fn write_failure_with_live_pty_stays_sticky_in_status_bar() {
        let mut view = test_shell();
        insert_test_pty(&mut view, "sleep", &["5"]);
        // A PTY write failure with a live session used to vanish past a
        // repaint (placeholder-only error slot). Now it sticks in the
        // status bar, with no Retry (the run already owns a live PTY).
        view.app.set_error("pty write failed: broken pipe");
        assert!(!view.can_retry());
        assert_eq!(view.status_text(), "pty write failed: broken pipe");
        // Info flashes never supersede it; repeated repaints keep it.
        view.app.set_status("copied selection (4 chars)");
        assert_eq!(view.status_text(), "pty write failed: broken pipe");
        view.refresh();
        assert_eq!(view.status_text(), "pty write failed: broken pipe");
        // Explicit dismissal (`d`) restores the transient info line.
        assert_eq!(view.nav_action("d", false), NavAction::Dismiss);
        view.app.clear_error();
        assert_eq!(view.status_text(), "copied selection (4 chars)");
        assert_eq!(view.nav_action("d", false), NavAction::None);
    }

    #[test]
    fn next_spawn_success_clears_the_sticky_error() {
        let mut view = test_shell();
        view.app.start_new_session();
        let id = view.active_id().unwrap();
        view.app.set_error("failed to spawn `muse`: missing binary");
        assert_eq!(
            view.status_text(),
            "failed to spawn `muse`: missing binary · r: retry"
        );
        // A later successful spawn (same policy `spawn_queued` runs on its
        // Ok branch) supersedes the error without any dismissal click.
        view.note_spawn_success(&id);
        assert_eq!(view.app.error_text(), None);
        assert!(!view.can_retry());
    }

    #[test]
    fn typed_line_becomes_run_title_on_enter_without_pty() {
        let mut view = test_shell();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        // No PTY inserted: bytes go nowhere, but title tracking still runs.
        for c in ["f", "i", "x"] {
            view.forward_key(c, Some(c), false, false);
        }
        view.forward_key("enter", None, false, false);
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.title, "fix");
    }
}
