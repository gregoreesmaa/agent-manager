//! Run lifecycle: restart, close/kill, and two-step quit.
//!
//! Headless like the rest of the shell logic: dropping a dead PTY,
//! removing an entry plus its live child, and arming a dirty quit are
//! all plain state transitions covered without a window.

use super::shell::ShellView;

impl ShellView {
    /// Restart is offered exactly when the selected run's child exited:
    /// the most common lifecycle event, previously a dead end.
    pub(crate) fn can_restart(&self) -> bool {
        self.active_id()
            .as_ref()
            .and_then(|id| self.runs.get(id))
            .is_some_and(|run| run.exited())
    }

    /// Resume is offered for entries with no live run and no queued spawn:
    /// historic provider sessions seeded at startup, waiting for an
    /// explicit re-attach. Fresh runs (spawn queued) are excluded.
    pub(crate) fn can_resume(&self) -> bool {
        match self.active_id() {
            Some(id) => !self.runs.contains_key(&id) && !self.app.has_pending_spawn(),
            None => false,
        }
    }

    /// Restart the selected ended run (or resume a historic entry): drop
    /// the dead PTY if any (reaping the child), forget its output recency,
    /// pending input, and cached attention, and queue the run's spawn kind
    /// on the same id — `Resume` for historic entries, `New` for live
    /// runs. Title, links, and transcript are kept: restart resumes the
    /// run's story, it does not archive it. No-op unless the active child
    /// exited or the entry awaits resume (live runs are never killed by
    /// accident; per-run close is separate).
    pub(crate) fn restart_run(&mut self) {
        if !self.can_restart() && !self.can_resume() {
            return;
        }
        if let Some(id) = self.active_id() {
            self.runs.remove(&id);
            if let Some(s) = self.app.sessions.iter_mut().find(|s| s.id == id) {
                s.pending_input.clear();
            }
            self.clear_selection();
            let kind = self.app.respawn_kind(&id);
            self.app.retry_spawn(kind);
        }
    }

    /// Close (kill) the selected run: drop its live PTY — `Drop` kills
    /// and reaps the child so no zombie survives — and remove its entry
    /// (pending input dies with it). Immediate and single-step;
    /// quitting the whole app is what asks. No-op when empty.
    pub(crate) fn close_run(&mut self) {
        if let Some(id) = self.active_id() {
            self.runs.remove(&id);
            self.clear_selection();
            if let Some(title) = self.app.remove_session(&id) {
                self.app.set_status(format!("closed '{title}'"));
                // The closed run must not resurrect from the last persist.
                self.persist_runs();
            }
        }
    }

    /// True when quitting must confirm first: any Working/Attention run
    /// or any live (non-exited) PTY. Empty/idle shells quit instantly.
    pub(crate) fn confirm_quit_required(&self) -> bool {
        self.app.needs_quit_confirm() || self.runs.values().any(|run| !run.exited())
    }

    /// Two-step quit: returns true when the app should exit now. The
    /// first call with dirty runs only arms (with a hint); the second
    /// quits. Safe states quit on the first call and never arm.
    /// Best-effort run-state persist (issue #26): a failed save just
    /// means the next start falls back to historic discovery alone.
    /// Unit tests only advance the throttle clock: the save/load
    /// roundtrip is covered in `persist.rs` with temp paths, so `cargo
    /// test` never rewrites the developer's live runs file.
    pub(crate) fn persist_runs(&mut self) {
        self.last_persist = Some(std::time::Instant::now());
        #[cfg(not(test))]
        let _ = crate::persist::save_sessions(&self.app.sessions);
    }

    pub(crate) fn request_quit(&mut self) -> bool {
        if !self.confirm_quit_required() {
            self.persist_runs();
            return true;
        }
        if self.quit_armed {
            self.persist_runs();
            return true;
        }
        self.quit_armed = true;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::nav::NavAction;
    use super::super::runs::{insert_test_pty, test_shell, Run};
    use super::super::shell::ShellView;

    #[test]
    fn restart_is_offered_only_for_exited_runs_and_keeps_the_run() {
        use std::time::Duration;

        let mut view = test_shell();
        assert!(!view.can_restart());
        assert_eq!(view.nav_action("r", false), NavAction::None);
        view.restart_run();
        assert!(view.app.take_pending_spawn().is_none());

        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        // No PTY and no queued spawn: the run awaits (re)start — `r`
        // offers it instead of staying a dead end.
        assert!(!view.can_restart());
        assert!(view.can_resume());
        assert_eq!(view.nav_action("r", false), NavAction::Restart);
        assert_eq!(
            view.status_text(),
            "run ready · r: start · n: new · ?: help · q: quit"
        );
        // Live child: restart is never offered (no accidental kills).
        let live =
            crate::embedded::EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.runs.insert(id.clone(), Run::new(live));
        assert!(!view.can_restart());
        assert!(!view.can_resume());
        assert_eq!(view.nav_action("r", false), NavAction::None);
        view.restart_run();
        assert!(view.runs.contains_key(&id));
        assert!(view.app.take_pending_spawn().is_none());

        // An exited child flips the offer on: status hint plus `r`.
        let dead = crate::embedded::EmbeddedPty::spawn("true", &[], 80, 24).unwrap();
        view.runs.insert(id.clone(), Run::new(dead));
        for _ in 0..100 {
            view.refresh();
            if view.can_restart() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(view.can_restart());
        assert_eq!(view.nav_action("r", false), NavAction::Restart);
        assert_eq!(
            view.status_text(),
            "run ended · r: restart · n: new · ?: help · q: quit"
        );

        // Restart drops the dead PTY and re-queues on the same id,
        // keeping the run's title (no new entry, no archive).
        let title_before = view.app.selected_session().unwrap().title.clone();
        view.restart_run();
        assert!(!view.runs.contains_key(&id));
        assert_eq!(
            view.app.take_pending_spawn(),
            Some(crate::embedded::SpawnKind::New)
        );
        assert_eq!(view.active_id().as_deref(), Some(id.as_str()));
        assert_eq!(view.app.selected_session().unwrap().title, title_before);
        assert!(!view.can_restart());
    }

    #[test]
    fn historic_entries_resume_the_provider_session() {
        use crate::app::{ChatSession, Status};
        // Seeded entry (as main.rs builds from discover_sessions): title,
        // links, and transcript present, no live PTY.
        let historic = ChatSession {
            id: "hist-1".into(),
            title: "old work".into(),
            project: "muse".into(),
            status: Status::Idle,
            last_active: 1,
            pr_links: vec!["https://github.com/acme/app/pull/9".into()],
            related_links: vec![],
            links_truncated: false,
            transcript: vec![],
            transcript_truncated: false,
            provider_session_id: Some("sess-abc".into()),
            title_locked: true,
            pending_input: String::new(),
        };
        let mut view = ShellView::new_with_sessions(vec![historic]);
        // Visible without any run; resume offered; status names it.
        assert_eq!(view.app.sessions.len(), 1);
        assert!(view.runs.is_empty());
        assert!(!view.can_restart());
        assert!(!view.can_retry());
        assert!(view.can_resume());
        assert_eq!(view.nav_action("r", false), NavAction::Restart);
        assert_eq!(
            view.status_text(),
            "historic run · r: resume · n: new · ?: help · q: quit"
        );
        // Restart re-attaches the provider session on the same entry:
        // title and links kept, only the child is new.
        view.restart_run();
        assert_eq!(
            view.app.take_pending_spawn(),
            Some(crate::embedded::SpawnKind::Resume {
                session_id: "sess-abc".to_string()
            })
        );
        assert_eq!(view.active_id().as_deref(), Some("hist-1"));
        assert_eq!(view.app.selected_session().unwrap().title, "old work");
        assert_eq!(view.app.selected_session().unwrap().pr_links.len(), 1);
    }

    #[test]
    fn safe_quit_is_instant_dirty_quit_arms_then_quits() {
        // Empty shell quits instantly and never arms.
        let mut view = test_shell();
        assert!(!view.confirm_quit_required());
        assert!(view.request_quit());
        assert!(!view.quit_armed);
        // A live PTY forces the two-step: arm first, quit second.
        insert_test_pty(&mut view, "sleep", &["5"]);
        assert!(view.confirm_quit_required());
        assert!(!view.request_quit());
        assert!(view.quit_armed);
        assert_eq!(
            view.status_text(),
            "Live runs active — q again to quit · any other key cancels"
        );
        assert!(view.request_quit());
        // Any other key disarms back to the hint.
        let mut view2 = test_shell();
        insert_test_pty(&mut view2, "sleep", &["5"]);
        assert!(!view2.request_quit());
        view2.nav_action("j", false);
        assert!(!view2.quit_armed);
        assert!(!view2.request_quit());
    }

    #[test]
    fn close_kills_run_entry_and_pty_without_touching_neighbors() {
        let mut view = test_shell();
        insert_test_pty(&mut view, "sleep", &["5"]);
        insert_test_pty(&mut view, "sleep", &["5"]);
        view.app.focus_nav();
        assert_eq!(view.app.sessions.len(), 2);
        assert_eq!(view.app.selected, 1);
        assert_eq!(view.nav_action("x", false), NavAction::Close);
        view.close_run();
        // The selected run is gone — entry and PTY — the neighbor keeps
        // both, and the flash names the closed run.
        assert_eq!(view.app.sessions.len(), 1);
        assert_eq!(view.app.sessions[0].id, "run-1");
        assert_eq!(view.runs.len(), 1);
        assert!(view.runs.contains_key("run-1"));
        assert!(view.status_text().contains("closed"));
        // Closing the last run empties the list without panicking.
        view.close_run();
        assert!(view.app.sessions.is_empty());
        assert!(view.runs.is_empty());
        assert!(!view.confirm_quit_required());
    }
}
