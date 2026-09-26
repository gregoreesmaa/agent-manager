//! Input handling for [`super::shell::ShellView`]: nav-focus key dispatch
//! plus the clipboard actions keys and mouse invoke.
//!
//! Key dispatch is pure state (headlessly testable): the caller applies
//! window focus and clipboard effects for the returned action. `muse`
//! captures keys if and only if focus is Terminal — nav keys never reach
//! the PTY. Parsed-link rows are keyboard-operable too: `o` cycles link
//! focus across the selected run's links, `Enter` copies the focused link,
//! and `PgUp`/`PgDn` page the run list.

use gpui::{App as GpuiApp, ClipboardItem};

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

/// In-app help entries: every nav key plus terminal-focus and mouse
/// bindings, so the full keymap no longer lives only in the README.
pub(crate) fn help_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("n", "new muse session"),
        ("j / k", "move selection between sessions"),
        ("↑ / ↓", "move selection between sessions"),
        ("PgUp / PgDn", "page the session list"),
        ("o", "cycle link focus across the selected run's links"),
        ("Enter", "copy the focused link, or type in muse when none"),
        ("i", "type in muse"),
        ("Tab", "switch sessions ↔ terminal focus"),
        ("y", "copy selection (or whole screen)"),
        ("p", "paste clipboard into muse"),
        ("r", "restart ended run / retry failed spawn"),
        ("x", "close (kill) the selected run"),
        ("d", "dismiss the sticky error"),
        ("?", "toggle this help"),
        ("Esc", "back to sessions · quit from sessions"),
        ("q", "quit (confirms first with live runs)"),
        ("drag", "select terminal text (copy-on-select)"),
        ("Cmd+C", "copy selection (or screen)"),
        ("Cmd/Ctrl+V", "paste clipboard"),
    ]
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
            // `?` toggles the in-app help panel. `/` is an alias for
            // keyboards where `?` needs shift or is hard to discover.
            ("?", _) | ("/", false) => {
                self.show_help = !self.show_help;
                NavAction::None
            }
            _ => NavAction::None,
        };
        // Any key other than a quit intent cancels an armed quit.
        if action != NavAction::Quit {
            self.quit_armed = false;
        }
        action
    }

    /// Copy the mouse selection when one exists, else the whole active
    /// screen, to the system clipboard.
    pub(crate) fn copy_screen(&mut self, cx: &mut GpuiApp) {
        if let Some(text) = self.selected_text() {
            let chars = text.chars().count();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.app
                .set_status(format!("copied selection ({chars} chars)"));
            return;
        }
        if let Some(view) = self.active_view() {
            let text = view.screen.contents();
            let lines = text.lines().count();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.app
                .set_status(format!("yanked {lines} lines to clipboard"));
        }
    }

    /// Paste the system clipboard into the active PTY as typed bytes.
    pub(crate) fn paste_clipboard(&mut self, cx: &mut GpuiApp) {
        let Some(item) = cx.read_from_clipboard() else {
            self.app.set_status("clipboard is empty");
            return;
        };
        let Some(text) = item.text() else {
            self.app.set_status("clipboard has no text");
            return;
        };
        if text.is_empty() {
            self.app.set_status("clipboard is empty");
            return;
        }
        let Some(id) = self.active_id() else {
            return;
        };
        if let Some(run) = self.runs.get_mut(&id) {
            if let Err(e) = run.pty.write_input(text.as_bytes()) {
                self.app.set_error(e.to_string());
            } else {
                let chars = text.chars().count();
                self.app.set_status(format!("pasted {chars} chars"));
            }
        }
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
    fn help_toggle_and_keymap_coverage() {
        // Issue #7: `?` (and `/` alias) toggles help in nav focus; the
        // panel documents every nav key plus terminal/mouse bindings, and
        // every status hint advertises `?`.
        let mut view = test_shell();
        assert!(!view.show_help);
        assert_eq!(view.nav_action("?", false), NavAction::None);
        assert!(view.show_help);
        assert_eq!(view.nav_action("?", false), NavAction::None);
        assert!(!view.show_help);
        assert_eq!(view.nav_action("/", false), NavAction::None);
        assert!(view.show_help);
        assert_eq!(view.nav_action("/", false), NavAction::None);
        assert!(!view.show_help);
        let entries = super::help_entries();
        assert!(entries.len() >= 10);
        let keys: Vec<&str> = entries.iter().map(|(k, _)| *k).collect();
        for needed in [
            "n",
            "j / k",
            "PgUp / PgDn",
            "o",
            "Enter",
            "i",
            "Tab",
            "y",
            "p",
            "r",
            "x",
            "d",
            "?",
            "q",
            "Esc",
            "drag",
            "Cmd+C",
            "Cmd/Ctrl+V",
        ] {
            assert!(keys.contains(&needed), "help documents {needed}");
        }
        for (k, what) in &entries {
            assert!(!k.is_empty() && !what.is_empty());
        }
        // Every status hint advertises `?`: empty, nav, and terminal.
        let mut view = test_shell();
        view.app.focus_nav();
        assert!(
            view.status_text().contains('?'),
            "empty hint advertises help: {}",
            view.status_text()
        );
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        view.app.focus_nav();
        assert!(
            view.status_text().contains('?'),
            "nav hint advertises help: {}",
            view.status_text()
        );
        view.app.focus_terminal();
        assert!(
            view.status_text().contains('?'),
            "terminal hint advertises help: {}",
            view.status_text()
        );
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
