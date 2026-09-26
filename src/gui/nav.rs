//! Nav-focus key dispatch for [`super::shell::ShellView`].
//!
//! Pure state (headlessly testable): the caller applies window focus and
//! clipboard effects for the returned action. `muse` captures keys if and
//! only if focus is Terminal — nav keys never reach the PTY. Parsed-link
//! rows are keyboard-operable too: `o` cycles link focus across the
//! selected run's links, `Enter` copies the focused link, and `PgUp`/`PgDn`
//! page the run list.

use super::shell::ShellView;

/// Outcome of a nav-focus keypress: state changes apply immediately,
/// window/clipboard effects are applied by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavAction {
    Quit,
    FocusTerm,
    Copy,
    Paste,
    Retry,
    Restart,
    Close,
    Dismiss,
    /// Copy the keyboard-focused parsed link (see `link_cursor`).
    CopyLink,
    None,
}

impl ShellView {
    /// URL of the keyboard-focused parsed link, if the selected run has
    /// one at [`Self::link_cursor`].
    pub(crate) fn focused_link_url(&self) -> Option<String> {
        let cursor = self.link_cursor?;
        let id = self.active_id()?;
        self.app
            .sessions
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.pr_links.get(cursor))
            .cloned()
    }

    /// Advance link focus through the selected run's parsed links,
    /// wrapping back to unfocused after the last one (so Enter can focus
    /// the terminal again without moving the run selection).
    pub(crate) fn cycle_link_focus(&mut self) {
        let count = match self.active_id() {
            Some(id) => self
                .app
                .sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.pr_links.len())
                .unwrap_or(0),
            None => 0,
        };
        if count == 0 {
            self.link_cursor = None;
            return;
        }
        self.link_cursor = match self.link_cursor {
            None => Some(0),
            Some(i) if i + 1 < count => Some(i + 1),
            _ => None,
        };
    }

    /// Nav-focus key dispatch, pure state (headlessly testable). The caller
    /// applies window focus and clipboard effects for the returned action.
    /// `muse` captures keys if and only if focus is Terminal — nav keys
    /// never reach the PTY.
    pub(crate) fn nav_action(&mut self, key: &str, ctrl: bool) -> NavAction {
        let action = match (key, ctrl) {
            ("q", false) | ("escape", _) => NavAction::Quit,
            ("j", false) | ("down", _) => {
                self.app.select_next();
                self.clear_selection();
                self.link_cursor = None;
                NavAction::None
            }
            ("k", false) | ("up", _) => {
                self.app.select_prev();
                self.clear_selection();
                self.link_cursor = None;
                NavAction::None
            }
            ("pagedown", _) => {
                self.app.select_page_next();
                self.clear_selection();
                self.link_cursor = None;
                NavAction::None
            }
            ("pageup", _) => {
                self.app.select_page_prev();
                self.clear_selection();
                self.link_cursor = None;
                NavAction::None
            }
            ("n", false) => {
                self.app.start_new_session();
                self.clear_selection();
                self.link_cursor = None;
                NavAction::FocusTerm
            }
            ("o", false) => {
                self.cycle_link_focus();
                NavAction::None
            }
            ("i", false) => NavAction::FocusTerm,
            ("enter", _) if self.focused_link_url().is_some() => NavAction::CopyLink,
            ("enter", _) => NavAction::FocusTerm,
            ("y", false) => NavAction::Copy,
            ("p", false) => NavAction::Paste,
            ("r", false) if self.can_restart() => NavAction::Restart,
            ("r", false) if self.can_retry() => NavAction::Retry,
            ("x", false) => NavAction::Close,
            ("d", false) if self.app.error_text().is_some() => NavAction::Dismiss,
            _ => NavAction::None,
        };
        // Any key other than a quit intent cancels an armed quit.
        if action != NavAction::Quit {
            self.quit_armed = false;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::super::runs::test_shell;
    use super::NavAction;

    #[test]
    fn nav_keys_never_reach_the_pty_only_terminal_focus_forwards() {
        let mut view = test_shell();
        // Nav: n creates a run and yields FocusTerm; q yields Quit.
        assert_eq!(view.nav_action("n", false), NavAction::FocusTerm);
        assert_eq!(view.app.sessions.len(), 1);
        assert!(view.app.is_terminal_focused());
        assert_eq!(view.nav_action("q", false), NavAction::Quit);
        // y copies, p pastes; unknown keys are inert.
        assert_eq!(view.nav_action("y", false), NavAction::Copy);
        assert_eq!(view.nav_action("p", false), NavAction::Paste);
        assert_eq!(view.nav_action("z", false), NavAction::None);
        assert_eq!(view.nav_action("j", true), NavAction::None);
    }

    #[test]
    fn pageup_pagedown_page_the_run_list() {
        let mut view = test_shell();
        // No runs: paging is inert.
        assert_eq!(view.nav_action("pagedown", false), NavAction::None);
        assert_eq!(view.nav_action("pageup", false), NavAction::None);
        for _ in 0..8 {
            view.app.start_new_session();
            let _ = view.app.take_pending_spawn();
        }
        view.app.focus_nav();
        view.app.selected = 0;
        assert_eq!(view.nav_action("pagedown", false), NavAction::None);
        assert_eq!(view.app.selected, crate::app::PAGE_STEP);
        assert_eq!(view.nav_action("pagedown", false), NavAction::None);
        assert_eq!(view.app.selected, 7);
        assert_eq!(view.nav_action("pageup", false), NavAction::None);
        assert_eq!(view.app.selected, 7 - crate::app::PAGE_STEP);
        view.app.selected = 1;
        assert_eq!(view.nav_action("pageup", false), NavAction::None);
        assert_eq!(view.app.selected, 0);
    }

    #[test]
    fn o_cycles_link_focus_and_enter_copies_the_focused_link() {
        let mut view = test_shell();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        view.app.focus_nav();
        // No links: `o` stays unfocused, Enter focuses the terminal.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
        assert_eq!(view.nav_action("enter", false), NavAction::FocusTerm);

        let id = view.active_id().unwrap();
        let s = view.app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        s.pr_links
            .push("https://github.com/acme/app/pull/42".to_string());
        s.pr_links
            .push("https://github.com/acme/app/pull/43".to_string());

        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(0));
        assert_eq!(view.nav_action("enter", false), NavAction::CopyLink);
        assert_eq!(
            view.focused_link_url().as_deref(),
            Some("https://github.com/acme/app/pull/42")
        );
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(1));
        assert_eq!(view.nav_action("enter", false), NavAction::CopyLink);
        // Past the last link focus wraps back to unfocused: Enter types.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
        assert_eq!(view.nav_action("enter", false), NavAction::FocusTerm);
        // `i` always focuses the terminal, even with a link focused.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(0));
        assert_eq!(view.nav_action("i", false), NavAction::FocusTerm);
        // Moving the run selection clears link focus.
        assert_eq!(view.nav_action("j", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
    }
}
