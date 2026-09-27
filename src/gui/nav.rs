//! Input handling for [`super::shell::ShellView`]: nav-focus key dispatch
//! plus the clipboard actions keys and mouse invoke.
//!
//! Key dispatch is pure state (headlessly testable): the caller applies
//! window focus and clipboard effects for the returned action. `muse`
//! captures keys if and only if focus is Terminal — nav keys never reach
//! the PTY. Parsed-link rows are keyboard-operable too: `o` cycles link
//! focus across the selected run's links, `Enter` opens the focused link
//! in the default browser (and copies it), `y` copies it without opening,
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
    /// Open the keyboard-focused parsed link in the default browser
    /// (issue #49). The caller also copies it, so open and yank stay
    /// one key apart: `Enter` opens + copies, `y` copies only.
    OpenLink,
    /// Advance the theme choice (dark → light → system); the caller
    /// applies, persists, and flashes it.
    CycleTheme,
    /// Export the selected run's visible text plus links to markdown.
    Export,
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
        (
            "Shift+PgUp / Shift+PgDn",
            "scroll the run's retained output (pager)",
        ),
        ("o", "cycle link focus across the selected run's links"),
        (
            "Enter",
            "open the focused link in the browser (+ copy), or type in muse when none",
        ),
        ("i", "type in muse"),
        ("Tab", "switch sessions ↔ terminal focus"),
        (
            "w",
            "new session in a chosen folder (blank = current folder)",
        ),
        (
            "y",
            "copy the focused link, else selection (or whole screen)",
        ),
        (
            "click link",
            "open in browser + copy · ⌘/Ctrl-click copies only",
        ),
        ("p", "paste clipboard into muse"),
        ("e", "export selected run to markdown (local file)"),
        ("r", "restart ended run / retry failed spawn"),
        ("x", "close (kill) the selected run"),
        ("d", "dismiss the sticky error"),
        ("/", "filter sessions by title substring"),
        ("+ / -", "terminal font size (saved locally)"),
        ("[ / ]", "sessions panel width (saved locally)"),
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
    /// one at [`Self::link_cursor`]. The cursor walks PR links first,
    /// then related (issue/commit/commit) links, so every openable row
    /// the panel shows is keyboard-reachable (issue #49).
    pub(crate) fn focused_link_url(&self) -> Option<String> {
        let cursor = self.link_cursor?;
        let id = self.active_id()?;
        self.app.sessions.iter().find(|s| s.id == id).and_then(|s| {
            s.pr_links
                .iter()
                .chain(s.related_links.iter())
                .nth(cursor)
                .cloned()
        })
    }

    /// True when the keyboard-focused link is a PR link (rather than a
    /// related link): picks the "opened + copied PR link" vs "… link"
    /// status wording.
    pub(crate) fn focused_link_is_pr(&self) -> bool {
        let Some(cursor) = self.link_cursor else {
            return false;
        };
        let Some(id) = self.active_id() else {
            return false;
        };
        self.app
            .sessions
            .iter()
            .find(|s| s.id == id)
            .is_some_and(|s| cursor < s.pr_links.len())
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
                .map(|s| s.pr_links.len() + s.related_links.len())
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
                // Capped by the live-run policy (issue #31): a refused
                // 11th run stays in the list so the flash is readable.
                if self.request_new_run() {
                    NavAction::FocusTerm
                } else {
                    NavAction::None
                }
            }
            ("o", false) => {
                self.cycle_link_focus();
                NavAction::None
            }
            ("i", false) => NavAction::FocusTerm,
            ("enter", _) if self.focused_link_url().is_some() => NavAction::OpenLink,
            ("enter", _) => NavAction::FocusTerm,
            // Yank (issue #49): with a link focused this copies the
            // link without opening it; otherwise it copies the
            // selection/screen exactly as before.
            ("y", false) if self.focused_link_url().is_some() => NavAction::CopyLink,
            ("y", false) => NavAction::Copy,
            ("p", false) => NavAction::Paste,
            ("e", false) => NavAction::Export,
            ("r", false) if self.can_restart() => NavAction::Restart,
            ("r", false) if self.can_retry() => NavAction::Retry,
            ("r", false) if self.can_resume() => NavAction::Restart,
            ("x", false) => NavAction::Close,
            ("d", false) if self.app.error_text().is_some() => NavAction::Dismiss,
            // `?` toggles the in-app help panel. (`/` used to be an alias;
            // since issue #29 it opens title-filter capture instead — the
            // shell routes it before `nav_action`, so it never reaches here.)
            ("?", _) => {
                self.show_help = !self.show_help;
                NavAction::None
            }
            ("t", false) => NavAction::CycleTheme,
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
        // A paged run copies from the visible pager slice, not the live
        // grid underneath (issue #25).
        if let Some(text) = self.selected_pager_text().or_else(|| self.selected_text()) {
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

    /// Platform command that opens `url` in the default browser
    /// (issue #49): `open` on macOS, `xdg-open` on Linux/Unix, `cmd /c
    /// start` on Windows. Pure construction so tests assert the exact
    /// URL is passed through without launching anything.
    pub(crate) fn browser_command(url: &str) -> std::process::Command {
        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new("open");
            cmd.arg(url);
            cmd
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(url);
            cmd
        }
        #[cfg(target_os = "windows")]
        {
            let mut cmd = std::process::Command::new("cmd");
            cmd.args(["/c", "start", "", url]);
            cmd
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(url);
            cmd
        }
    }

    /// Open `url` in the default browser (issue #49). Best effort like
    /// every other local spawn here: the caller reports the error in
    /// the status line instead of failing.
    pub(crate) fn open_url(url: &str) -> Result<(), String> {
        Self::browser_command(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Open-or-copy split for a link click (issue #49): a plain click
    /// opens the URL in the browser and copies it ("opened + copied");
    /// a modifier-click (Cmd/Ctrl/Shift) copies only ("copied"). The
    /// caller applies the clipboard write and the status flash for the
    /// returned outcome; opening itself runs here.
    pub(crate) fn activate_link(
        &mut self,
        url: &str,
        is_pr: bool,
        copy_only: bool,
        cx: &mut GpuiApp,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(url.to_string()));
        let kind = if is_pr { "PR link" } else { "link" };
        if copy_only {
            self.app.set_status(format!("copied {kind}"));
            return;
        }
        match Self::open_url(url) {
            Ok(()) => self.app.set_status(format!("opened + copied {kind}")),
            Err(e) => self
                .app
                .set_status(format!("copied {kind} (open failed: {e})")),
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
        // Issue #7: `?` toggles help in nav focus; the panel documents
        // every nav key plus terminal/mouse bindings, and every status
        // hint advertises `?`. (`/` was the discoverability alias; since
        // issue #29 it opens title-filter capture, routed in the shell
        // before `nav_action`, so it must not toggle help here.)
        let mut view = test_shell();
        assert!(!view.show_help);
        assert_eq!(view.nav_action("?", false), NavAction::None);
        assert!(view.show_help);
        assert_eq!(view.nav_action("?", false), NavAction::None);
        assert!(!view.show_help);
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
    fn t_cycles_the_theme_choice() {
        // Issue #34: `t` dispatches CycleTheme without mutating (the
        // caller cycles, applies, and persists); the cycle itself starts
        // at the default system choice.
        use crate::config::ThemePreference;
        let mut view = test_shell();
        assert_eq!(view.nav_action("t", false), NavAction::CycleTheme);
        assert_eq!(view.app.cycle_theme(), ThemePreference::Dark);
        assert_eq!(view.app.cycle_theme(), ThemePreference::Light);
        assert_eq!(view.app.cycle_theme(), ThemePreference::System);
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
    fn o_cycles_link_focus_enter_opens_y_copies() {
        // Issue #49: Enter opens the focused link (OpenLink), `y`
        // copies it without opening (CopyLink); with no link focused
        // both fall back to the old behavior.
        let mut view = test_shell();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        view.app.focus_nav();
        // No links: `o` stays unfocused, Enter focuses the terminal,
        // `y` copies the screen.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
        assert_eq!(view.nav_action("enter", false), NavAction::FocusTerm);
        assert_eq!(view.nav_action("y", false), NavAction::Copy);

        let id = view.active_id().unwrap();
        let s = view.app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        s.pr_links
            .push("https://github.com/acme/app/pull/42".to_string());
        s.pr_links
            .push("https://github.com/acme/app/pull/43".to_string());
        s.related_links
            .push("https://github.com/acme/app/issues/9".to_string());

        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(0));
        assert!(view.focused_link_is_pr());
        assert_eq!(view.nav_action("enter", false), NavAction::OpenLink);
        assert_eq!(view.nav_action("y", false), NavAction::CopyLink);
        assert_eq!(
            view.focused_link_url().as_deref(),
            Some("https://github.com/acme/app/pull/42")
        );
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(1));
        assert_eq!(view.nav_action("enter", false), NavAction::OpenLink);
        // Link focus walks related links after the PRs, so every
        // openable row is keyboard-reachable.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(2));
        assert!(!view.focused_link_is_pr());
        assert_eq!(
            view.focused_link_url().as_deref(),
            Some("https://github.com/acme/app/issues/9")
        );
        assert_eq!(view.nav_action("enter", false), NavAction::OpenLink);
        assert_eq!(view.nav_action("y", false), NavAction::CopyLink);
        // Past the last link focus wraps back to unfocused: Enter
        // types, `y` copies the screen again.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
        assert_eq!(view.nav_action("enter", false), NavAction::FocusTerm);
        assert_eq!(view.nav_action("y", false), NavAction::Copy);
        // `i` always focuses the terminal, even with a link focused.
        assert_eq!(view.nav_action("o", false), NavAction::None);
        assert_eq!(view.link_cursor, Some(0));
        assert_eq!(view.nav_action("i", false), NavAction::FocusTerm);
        // Moving the run selection clears link focus.
        assert_eq!(view.nav_action("j", false), NavAction::None);
        assert_eq!(view.link_cursor, None);
    }

    #[test]
    fn browser_command_passes_the_exact_url_through() {
        // Issue #49 falsifiable check: activating a PR row opens that
        // exact `github.com/<owner>/<repo>/pull/<n>` URL. Construction
        // is pure, so this asserts the argv without launching anything.
        let url = "https://github.com/acme/app/pull/42";
        let cmd = super::ShellView::browser_command(url);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.contains(&url.to_string()),
            "exact URL in argv: {args:?}"
        );
        let program = cmd.get_program().to_string_lossy().into_owned();
        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "xdg-open");
    }
}
