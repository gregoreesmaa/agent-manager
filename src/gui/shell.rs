//! gpui shell: runs panel, faithful terminal pane, status bar.
//!
//! Thin by design (regression shield): all parsing, key encoding, and style
//! mapping live in framework-free modules ([`super::terminal`],
//! [`super::keys`], [`crate::app`]); this view only lays out gpui elements,
//! forwards events, and pumps PTYs. The shell logic itself lives in focused
//! modules — [`super::runs`] (the single `Run` struct), [`super::spawn`]
//! (spawn/key paths), [`super::lifecycle`] (restart/close/quit),
//! [`super::pump`] (dirty-gated pump), [`super::nav`] (key dispatch,
//! clipboard), [`super::runs_panel`] (sessions panel),
//! [`super::terminal_pane`] (terminal render, selection),
//! [`super::layout`] (responsive geometry), [`super::view`] (window
//! composition) — none over ~400 lines.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App as GpuiApp, Bounds, ClipboardItem, FocusHandle, KeyDownEvent, Pixels, Window};

use crate::app::{App, ChatSession};
use crate::embedded::LiveView;

use super::layout::NARROW_BREAKPOINT;
use super::nav::NavAction;
use super::runs::Run;
use super::terminal::{CellPos, Rgb8};

impl ShellView {
    /// Terminal font size in points (issue #35: user-overridable via
    /// `terminal.font_size`, default 13.0).
    pub(crate) fn term_font_size(&self) -> f32 {
        let size = self.app.terminal_config().font_size;
        if size > 0.0 {
            size
        } else {
            13.0
        }
    }
}
/// Pump cadence: the background task polls PTYs at 20 Hz; actual
/// repaints are dirty-gated (see `tick`), so idle costs ~zero.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(50);
/// Selection highlight behind terminal text (classic selection blue).
pub(crate) const SELECTION_BG: u32 = 0x264f78;
/// Default terminal foreground when the child requests the default color.
pub(crate) const DEFAULT_FG: Rgb8 = Rgb8(212, 212, 212);
/// Caret color for the emulated cursor cell.
pub(crate) const CURSOR_BG: Rgb8 = Rgb8(180, 180, 180);

pub struct ShellView {
    pub(crate) app: App,
    /// One live run per entry: the single map replaces the old parallel
    /// `ptys` / `last_output` / `pending_inputs` maps (see [`Run`]).
    pub(crate) runs: HashMap<String, Run>,
    /// Two-step quit arming: the first `q` with dirty runs only arms and
    /// hints; the second quits. Any other key disarms.
    pub(crate) quit_armed: bool,
    /// Keyboard-focused parsed link: index into the selected run's
    /// `pr_links`. `None` means no link is focused (Enter focuses the
    /// terminal). Cleared whenever the run selection moves.
    pub(crate) link_cursor: Option<usize>,
    /// In-app help panel visibility, toggled by `?` in nav focus.
    pub(crate) show_help: bool,
    pub(crate) list_focus: Option<FocusHandle>,
    pub(crate) term_focus: Option<FocusHandle>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) focused_once: bool,
    /// Window-space bounds of the terminal text, captured each frame so a
    /// mouse drag maps back to terminal cells for highlighting.
    pub(crate) term_text_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// Last measured monospace cell size, for drag→cell mapping and for
    /// painting the selection highlight behind the text.
    pub(crate) char_w: f32,
    pub(crate) line_h: f32,
    /// Font-system metrics cache (see `fit_pty`): the measured cells plus
    /// the configured `(font family, size bits)` they were measured for.
    /// The text system is app-global so resizes never re-measure — only a
    /// changed terminal font (issue #35) drops the entry.
    pub(crate) mono_metrics: Option<((String, u32), (f32, f32))>,
    /// Last rendered terminal frame, keyed by run id + screen fingerprint
    /// (see [`super::terminal_pane::TermFrameCache`]): unchanged screens
    /// skip the `screen_rows` + `layout_text` rebuild every frame.
    pub(crate) term_frame: super::terminal_pane::TermFrameCache,
    /// Mouse-drag selection in terminal cells (anchor, cursor). `None` while
    /// no drag is in progress / no selection exists.
    pub(crate) sel_anchor: Option<CellPos>,
    pub(crate) sel_active: Option<CellPos>,
    pub(crate) selecting: bool,
}

impl ShellView {
    /// Test-only shorthand: production starts from provider-seeded sessions
    /// via [`Self::new_with_sessions`].
    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_with_sessions(vec![])
    }

    /// Start with provider-discovered entries (historic attach): sessions
    /// appear in the list with their titles, links, and transcripts before
    /// any PTY exists; `r` re-attaches the selected one (`Resume`).
    pub fn new_with_sessions(sessions: Vec<ChatSession>) -> Self {
        Self {
            app: App::new(sessions),
            runs: HashMap::new(),
            quit_armed: false,
            link_cursor: None,
            show_help: false,
            list_focus: None,
            term_focus: None,
            cols: 100,
            rows: 30,
            focused_once: false,
            term_text_bounds: Rc::new(RefCell::new(None)),
            char_w: 8.0,
            line_h: 18.0,
            mono_metrics: None,
            term_frame: super::terminal_pane::TermFrameCache::default(),
            sel_anchor: None,
            sel_active: None,
            selecting: false,
        }
    }

    pub(crate) fn active_id(&self) -> Option<String> {
        self.app.selected_session().map(|s| s.id.clone())
    }

    pub(crate) fn active_view(&self) -> Option<LiveView<'_>> {
        self.active_id()
            .as_ref()
            .and_then(|id| self.runs.get(id))
            .map(|run| run.pty.view())
    }

    pub(crate) fn focus_term(&mut self, window: &mut Window) {
        self.app.focus_terminal();
        if let Some(h) = &self.term_focus {
            window.focus(h);
        }
    }

    pub(crate) fn focus_list(&mut self, window: &mut Window) {
        self.app.focus_nav();
        if let Some(h) = &self.list_focus {
            window.focus(h);
        }
    }

    pub(crate) fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut GpuiApp) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let key_char = ks.key_char.as_deref();
        let ctrl = ks.modifiers.control;
        let alt = ks.modifiers.alt;
        let platform = ks.modifiers.platform;
        let shift = ks.modifiers.shift;
        if shift && !ctrl && !platform && (key == "pageup" || key == "pagedown") {
            // Scrollback pager (issue #25) in either focus: Shift+PgUp /
            // Shift+PgDn never types into `muse`. Repeats keep paging.
            self.page_scrollback(key == "pageup");
            window.refresh();
            return;
        }
        if ev.is_held {
            // Held-key repeats still type into the terminal; nav ignores them.
            if !self.app.is_terminal_focused() {
                return;
            }
        }
        if key == "tab" {
            self.app.toggle_focus();
            if self.app.is_terminal_focused() {
                self.focus_term(window);
            } else {
                self.focus_list(window);
            }
            window.refresh();
            return;
        }
        if self.app.is_terminal_focused() {
            // Typing means the user is staying: cancel an armed quit.
            self.quit_armed = false;
            if key == "escape" {
                self.clear_selection();
                self.focus_list(window);
                window.refresh();
                return;
            }
            // Dead pane (spawn failed, child exited, or historic entry with
            // no live PTY owning the keys): `r` retries/restarts/resumes
            // instead of typing into nothing. Retry stays gated on a
            // recorded failure so a fast first `r` still reaches a
            // starting child.
            if key.eq_ignore_ascii_case("r")
                && !ctrl
                && !platform
                && (self.can_restart() || self.can_retry() || self.can_resume())
            {
                if self.can_restart() || self.can_resume() {
                    self.restart_run();
                } else {
                    self.retry_spawn();
                }
                window.refresh();
                return;
            }
            // Clipboard shortcuts never reach `muse`: Cmd+C copies the mouse
            // selection (or screen), Cmd/Ctrl+V pastes. Ctrl+C still
            // interrupts (forwarded below).
            if platform && key.eq_ignore_ascii_case("c") {
                self.copy_screen(cx);
                window.refresh();
                return;
            }
            if (ctrl || platform) && key.eq_ignore_ascii_case("v") {
                self.paste_clipboard(cx);
                window.refresh();
                return;
            }
            self.forward_key(key, key_char, ctrl, alt);
            window.refresh();
            return;
        }
        if platform && key.eq_ignore_ascii_case("c") {
            self.copy_screen(cx);
            window.refresh();
            return;
        }
        if (ctrl || platform) && key.eq_ignore_ascii_case("v") {
            self.paste_clipboard(cx);
            window.refresh();
            return;
        }
        match self.nav_action(key, ctrl) {
            NavAction::Quit => {
                // Dirty shells arm first and quit on the second `q`;
                // safe shells quit instantly and never arm.
                if self.request_quit() {
                    cx.quit();
                }
                return;
            }
            NavAction::FocusTerm => self.focus_term(window),
            NavAction::Copy => self.copy_screen(cx),
            NavAction::Paste => self.paste_clipboard(cx),
            NavAction::Retry => {
                self.retry_spawn();
                self.focus_term(window);
            }
            NavAction::Restart => {
                self.restart_run();
                self.focus_term(window);
            }
            NavAction::Close => {
                self.close_run();
            }
            NavAction::Dismiss => {
                self.app.clear_error();
            }
            NavAction::CopyLink => {
                if let Some(url) = self.focused_link_url() {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    self.app.set_status("copied PR link");
                }
            }
            NavAction::CycleTheme => {
                // Issue #34: apply now, persist the choice, flash it.
                let pref = self.app.cycle_theme();
                let mode = pref.theme_mode(Some(window.appearance()));
                gpui_component::Theme::change(mode, None, cx);
                match self.app.save_config() {
                    Ok(()) => self.app.set_status(format!("theme: {}", pref.label())),
                    Err(e) => self
                        .app
                        .set_status(format!("theme: {} (not saved: {e})", pref.label())),
                }
            }
            NavAction::None => {}
        }
        window.refresh();
    }

    /// Wide (default) status hints. Test-only shorthand: production render
    /// always goes through [`Self::status_text_for_width`] with the live
    /// viewport width.
    #[cfg(test)]
    pub(crate) fn status_text(&self) -> String {
        self.status_text_for_width(f32::INFINITY)
    }

    /// Width-aware status text: narrow viewports (<700px) get compact key
    /// hints that fit beside the collapsed layout; errors, transient
    /// flashes, quit-arm, and ended-run lines are identical at every width
    /// (only the default key-hint lines compact — the bar also truncates
    /// with an ellipsis, so long messages never push the layout).
    pub(crate) fn status_text_for_width(&self, viewport_w: f32) -> String {
        // An armed quit outranks everything: the user asked to leave.
        if self.quit_armed {
            return "Live runs active — q again to quit · any other key cancels".to_string();
        }
        if let Some(msg) = self.app.status_text() {
            // Sticky errors keep their recovery hint while Retry applies.
            if self.app.error_text().is_some() && self.can_retry() {
                return format!("{msg} · r: retry");
            }
            return msg.to_string();
        }
        if self.can_restart() {
            return "run ended · r: restart · n: new · ?: help · q: quit".to_string();
        }
        if self.can_resume() {
            // Historic provider entries re-attach (`r: resume`); runs that
            // never started offer a plain start.
            let historic = self
                .active_id()
                .as_ref()
                .and_then(|id| {
                    self.app
                        .sessions
                        .iter()
                        .find(|s| &s.id == id)
                        .and_then(|s| s.provider_session_id.clone())
                })
                .is_some();
            if historic {
                return "historic run · r: resume · n: new · ?: help · q: quit".to_string();
            }
            return "run ready · r: start · n: new · ?: help · q: quit".to_string();
        }
        let narrow = viewport_w < NARROW_BREAKPOINT;
        if self.app.is_terminal_focused() {
            if narrow {
                "typing · Tab/Esc: sessions · Cmd+C: copy · Cmd+V: paste · ?: help".to_string()
            } else {
                "typing in muse · Tab/Esc: sessions · drag: select · Cmd+C: copy · Cmd/Ctrl+V: paste · ?: help"
                    .to_string()
            }
        } else if self.app.sessions.is_empty() {
            "n: new muse · ?: help · q: quit".to_string()
        } else if narrow {
            "n: new · j/k: move · o/Enter: link · Tab: type · x: close · y/p: copy/paste · t: theme · ?: help · q: quit"
                .to_string()
        } else {
            "n: new · j/k: move · PgUp/PgDn: page · o/Enter: copy link · Tab/i: type · x: close · drag: select · y: copy · p: paste · t: theme · ?: help · q: quit"
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpui_version_is_pinned() {
        // Fails loudly on upgrade: review gpui API changes consciously.
        // Manifest-relative so the test passes regardless of the
        // process working directory (CI, editors, `cargo test -p`).
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
            .expect("Cargo.lock readable in tests");
        let mut lines = lock.lines();
        let mut found = false;
        while let Some(line) = lines.next() {
            if line.trim() == r#"name = "gpui""# {
                let version = lines
                    .next()
                    .expect("version follows name")
                    .trim()
                    .to_string();
                assert_eq!(version, r#"version = "0.2.2""#, "gpui upgraded: review API");
                found = true;
                break;
            }
        }
        assert!(found, "gpui entry missing from Cargo.lock");
    }

    #[test]
    fn narrow_status_hints_stay_compact_but_complete() {
        // Issue #6: long key-hint lines break below ~700px; narrow widths
        // get compact hints that still name every essential key. A live
        // run keeps the view on the key-hint lines (the ready/resume and
        // empty states are width-invariant by design).
        let mut view = ShellView::new();
        crate::gui::runs::insert_test_pty(&mut view, "sleep", &["5"]);
        view.app.focus_nav();
        let full = view.status_text_for_width(1280.0);
        let narrow = view.status_text_for_width(600.0);
        assert_eq!(full, view.status_text(), "wide hints are unchanged");
        assert!(
            narrow.len() < full.len(),
            "narrow hints compact: {narrow:?} vs {full:?}"
        );
        for key in ["n:", "j/k", "o/Enter", "Tab", "x:", "t:", "q:"] {
            assert!(narrow.contains(key), "narrow hints keep {key}: {narrow:?}");
        }
        // Terminal-focus hints compact too.
        view.app.focus_terminal();
        let full_typing = view.status_text_for_width(1280.0);
        let narrow_typing = view.status_text_for_width(600.0);
        assert_eq!(full_typing, view.status_text());
        assert!(narrow_typing.len() < full_typing.len());
        assert!(narrow_typing.contains("Tab/Esc"));
        // Errors, flashes, and quit-arm are identical at every width.
        view.app
            .set_error("failed to spawn `muse`: missing binary".to_string());
        assert_eq!(
            view.status_text_for_width(600.0),
            view.status_text_for_width(1280.0)
        );
    }

    #[test]
    fn chrome_stays_on_the_gpui_02_component_line() {
        // The sessions chrome uses gpui-component 0.5.x, the last line built
        // on gpui 0.2.2. 0.6+ moved to the gpui-pre 0.3.6 fork and would
        // force a framework migration: fail loudly so that move is conscious.
        // Manifest-relative so the test passes regardless of the
        // process working directory (CI, editors, `cargo test -p`).
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
            .expect("Cargo.lock readable in tests");
        let mut lines = lock.lines();
        let mut found = false;
        while let Some(line) = lines.next() {
            if line.trim() == r#"name = "gpui-component""# {
                let version = lines
                    .next()
                    .expect("version follows name")
                    .trim()
                    .to_string();
                assert!(
                    version.starts_with(r#"version = "0.5."#),
                    "gpui-component left 0.5.x: review migration, got {version}"
                );
                found = true;
                break;
            }
        }
        assert!(found, "gpui-component entry missing from Cargo.lock");
    }
}
