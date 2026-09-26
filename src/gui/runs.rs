//! Live-run container: one [`Run`] per entry.
//!
//! The shell used to keep three parallel maps (`ptys`, `last_output`,
//! `pending_inputs`) plus an attention cache, all keyed by run id and
//! able to desync. A single `Run` struct per id removes that bug class:
//! spawning, pumping, keying, closing, and restarting all move one entry.

use std::time::Instant;

use crate::embedded::EmbeddedPty;

/// One live run: its PTY, output recency, and the cached attention bit
/// (unchanged screens skip re-scanning). The pending input line lives on
/// the [`crate::app::ChatSession`] instead: title tracking runs even
/// before the PTY exists (fast typing into a queued spawn) or after it
/// dies, so key handling never depends on the child being alive.
pub struct Run {
    pub pty: EmbeddedPty,
    pub last_output: Instant,
    pub attention: bool,
    /// Pager offset in lines up from the live bottom (issue #25): 0 is
    /// the live screen, positive shows retained history. Fresh output
    /// snaps it back to 0 (see the pump); paging never touches the PTY.
    pub scroll_offset: usize,
}

impl Run {
    pub fn new(pty: EmbeddedPty) -> Self {
        Self {
            pty,
            last_output: Instant::now(),
            attention: false,
            scroll_offset: 0,
        }
    }

    /// Feed queued output into the emulator, refreshing output recency.
    /// Returns true when new output arrived.
    pub fn pump(&mut self) -> bool {
        if self.pty.pump() {
            self.last_output = Instant::now();
            true
        } else {
            false
        }
    }

    /// True once the child has exited (last frame stays visible).
    pub fn exited(&self) -> bool {
        self.pty.view().exited
    }
}

#[cfg(test)]
pub(crate) fn test_shell() -> super::shell::ShellView {
    super::shell::ShellView::new()
}

/// Start a run in `view` backed by a real child process, returning its id.
/// Headless tests use tiny fake commands (`echo`, `printf`, `sleep`,
/// `true`) so they never need a real `muse`.
#[cfg(test)]
pub(crate) fn insert_test_pty(
    view: &mut super::shell::ShellView,
    program: &str,
    args: &[&str],
) -> String {
    view.app.start_new_session();
    let _ = view.app.take_pending_spawn();
    let id = view.active_id().unwrap();
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let pty = EmbeddedPty::spawn(program, &owned, 80, 24).unwrap();
    view.runs.insert(id.clone(), Run::new(pty));
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_pump_refreshes_recency_and_reports_freshness() {
        let mut run = Run::new(EmbeddedPty::spawn("echo", &["hi".to_string()], 80, 24).unwrap());
        let born = run.last_output;
        std::thread::sleep(std::time::Duration::from_millis(50));
        // echo writes promptly: the pump sees bytes and moves recency.
        let mut saw_fresh = false;
        for _ in 0..100 {
            if run.pump() {
                saw_fresh = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(saw_fresh);
        assert!(run.last_output >= born);
        assert!(!run.attention);
    }

    #[test]
    fn run_detects_exit() {
        let mut run = Run::new(EmbeddedPty::spawn("true", &[], 80, 24).unwrap());
        for _ in 0..100 {
            run.pump();
            if run.exited() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(run.exited());
    }
}
