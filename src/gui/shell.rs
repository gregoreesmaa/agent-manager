//! gpui shell: runs panel, faithful terminal pane, status bar.
//!
//! Thin by design (regression shield): all parsing, key encoding, and style
//! mapping live in framework-free modules ([`super::terminal`],
//! [`super::keys`], [`crate::app`]); this view only lays out gpui elements,
//! forwards events, and pumps PTYs. The shell logic itself lives in focused
//! modules — [`super::runs`] (the single `Run` struct), [`super::spawn`]
//! (spawn/key paths), [`super::lifecycle`] (restart/close/quit),
//! [`super::pump`] (dirty-gated pump), [`super::nav`] (key dispatch),
//! [`super::runs_panel`] (sessions panel), [`super::terminal_pane`]
//! (terminal render, selection, clipboard) — none over ~400 lines.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, px, rgb, App as GpuiApp, Bounds, ClipboardItem, Context, ElementId, FocusHandle,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Render, SharedString, Size, StatefulInteractiveElement,
    Styled, Window, WindowOptions,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::Sizable as _;

use crate::app::App;
use crate::embedded::LiveView;

use super::nav::NavAction;
use super::runs::Run;
use super::terminal::{CellPos, Rgb8};

/// Left runs panel width in pixels.
pub(crate) const LEFT_WIDTH: f32 = 264.0;
/// Status bar height in pixels.
pub(crate) const STATUS_HEIGHT: f32 = 28.0;
/// Viewport width below which the runs sidebar collapses so the terminal
/// pane gets the full width (issue #6: fixed 264px chrome broke below
/// ~700px).
pub const NARROW_BREAKPOINT: f32 = 700.0;
/// PTY grid floors: the smallest live grid `fit_pty` will ever report.
/// Shared with the window minimum size so the OS never lets the window
/// shrink past what the terminal can display.
pub const MIN_COLS: u16 = 20;
pub const MIN_ROWS: u16 = 10;
/// Fallback monospace metrics (same fallbacks as `mono_metrics`); used to
/// derive the static window minimum below.
const FALLBACK_CHAR_W: f32 = 8.0;
const FALLBACK_LINE_H: f32 = 18.0;
/// Estimated terminal-header height (title row, possibly + Restart button).
const HEADER_HEIGHT: f32 = 32.0;
/// Minimum window size, derived from the PTY floors: wide enough for
/// MIN_COLS beside the sidebar at fallback metrics, tall enough for
/// MIN_ROWS plus the header and status bar (424 x 240).
pub const MIN_WINDOW_WIDTH: f32 = LEFT_WIDTH + MIN_COLS as f32 * FALLBACK_CHAR_W;
pub const MIN_WINDOW_HEIGHT: f32 =
    MIN_ROWS as f32 * FALLBACK_LINE_H + HEADER_HEIGHT + STATUS_HEIGHT;

/// True when the viewport is wide enough to show the runs sidebar.
pub fn sidebar_visible_for_width(viewport_w: f32) -> bool {
    viewport_w >= NARROW_BREAKPOINT
}

/// Sidebar width actually consumed at this viewport width: zero when
/// collapsed so the terminal pane (and `fit_pty`) use the full width.
pub fn effective_sidebar_width(viewport_w: f32) -> f32 {
    if sidebar_visible_for_width(viewport_w) {
        LEFT_WIDTH
    } else {
        0.0
    }
}

/// PTY grid for an available pane size: the pure math behind `fit_pty`.
/// Floors match the window minimum (cols >= MIN_COLS, rows >= MIN_ROWS).
pub fn pty_grid_for(avail_w: f32, avail_h: f32, char_w: f32, line_h: f32) -> (u16, u16) {
    let cols = ((avail_w.max(200.0) / char_w.max(1.0)) as u16).clamp(MIN_COLS, 400);
    let rows = ((avail_h.max(120.0) / line_h.max(1.0)) as u16).clamp(MIN_ROWS, 200);
    (cols, rows)
}
/// Terminal font size in points.
pub(crate) const TERM_FONT_SIZE: f32 = 13.0;
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
    /// Mouse-drag selection in terminal cells (anchor, cursor). `None` while
    /// no drag is in progress / no selection exists.
    pub(crate) sel_anchor: Option<CellPos>,
    pub(crate) sel_active: Option<CellPos>,
    pub(crate) selecting: bool,
}

impl ShellView {
    pub fn new() -> Self {
        Self {
            app: App::new(vec![]),
            runs: HashMap::new(),
            quit_armed: false,
            link_cursor: None,
            list_focus: None,
            term_focus: None,
            cols: 100,
            rows: 30,
            focused_once: false,
            term_text_bounds: Rc::new(RefCell::new(None)),
            char_w: 8.0,
            line_h: 18.0,
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

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut GpuiApp) {
        if ev.is_held {
            // Held-key repeats still type into the terminal; nav ignores them.
            if !self.app.is_terminal_focused() {
                return;
            }
        }
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let key_char = ks.key_char.as_deref();
        let ctrl = ks.modifiers.control;
        let alt = ks.modifiers.alt;
        let platform = ks.modifiers.platform;
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
            // Dead pane (spawn failed or child exited, no live PTY owns
            // the keys): `r` retries/restarts instead of typing into
            // nothing. Retry stays gated on a recorded failure so a fast
            // first `r` still reaches a starting child.
            if key.eq_ignore_ascii_case("r")
                && !ctrl
                && !platform
                && (self.can_restart() || self.can_retry())
            {
                if self.can_restart() {
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
    fn status_text_for_width(&self, viewport_w: f32) -> String {
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
            return "run ended · r: restart · n: new · q: quit".to_string();
        }
        let narrow = viewport_w < NARROW_BREAKPOINT;
        if self.app.is_terminal_focused() {
            if narrow {
                "typing · Tab/Esc: sessions · Cmd+C: copy · Cmd+V: paste".to_string()
            } else {
                "typing in muse · Tab/Esc: sessions · drag: select · Cmd+C: copy · Cmd/Ctrl+V: paste"
                    .to_string()
            }
        } else if self.app.sessions.is_empty() {
            "n: new muse · q: quit".to_string()
        } else if narrow {
            "n: new · j/k: move · o/Enter: link · Tab: type · x: close · y/p: copy/paste · q: quit"
                .to_string()
        } else {
            "n: new · j/k: move · PgUp/PgDn: page · o/Enter: copy link · Tab/i: type · x: close · drag: select · y: copy · p: paste · q: quit"
                .to_string()
        }
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.list_focus.is_none() {
            self.list_focus = Some(cx.focus_handle());
            self.term_focus = Some(cx.focus_handle());
        }
        if !self.focused_once {
            self.focused_once = true;
            if let Some(h) = &self.list_focus {
                window.focus(h);
            }
        }
        self.refresh();
        self.fit_pty(window, cx);
        // Narrow layout (issue #6): below ~700px the sidebar collapses and
        // the terminal pane takes the full width; keyboard nav (j/k/Tab)
        // keeps switching runs while collapsed.
        let viewport_w = f32::from(window.viewport_size().width);
        let show_sidebar = sidebar_visible_for_width(viewport_w);

        let state = if let Some(v) = self.active_view() {
            if v.exited {
                "ended"
            } else if self.app.is_terminal_focused() {
                "typing"
            } else {
                "live"
            }
        } else {
            "idle"
        };
        let (cmd, note) = self
            .active_view()
            .map(|v| {
                (
                    v.header.to_string(),
                    v.exit_note.map(|n| format!(" {n}")).unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| ("muse".to_string(), String::new()));
        let title = format!("Muse [{state}] — {cmd}{note}");
        // Focus indicator: the pane that owns the keyboard gets the bright
        // title; the other dims. `muse` captures keys iff focus is Terminal.
        let typing = self.app.is_terminal_focused() && state != "idle" && state != "ended";
        let term_title_color = if typing { rgb(0xffd866) } else { rgb(0x888888) };

        let mut mid_row = div().flex().flex_row().flex_1();
        if show_sidebar {
            mid_row = mid_row.child(self.render_runs(cx));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x11111b))
            .track_focus(&self.list_focus.clone().unwrap())
            .id(ElementId::Name("app-root".into()))
            .on_key_down(cx.listener(|this, ev, window, cx| {
                this.on_key(ev, window, cx);
            }))
            .child(
                mid_row.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child({
                            // Ended-run recovery lives in the header:
                            // the title names the state, Restart reruns
                            // the same run id (keyboard `r`).
                            let mut header = div()
                                .px_2()
                                .py_1()
                                .text_color(term_title_color)
                                .text_sm()
                                .child(title);
                            if self.can_restart() {
                                header = header.child(
                                    Button::new(ElementId::Name("restart-run-btn".into()))
                                        .label("Restart (r)")
                                        .primary()
                                        .small()
                                        .on_click(cx.listener(|this, _ev, window, _cx| {
                                            this.restart_run();
                                            this.focus_term(window);
                                        })),
                                );
                            }
                            header
                        })
                        .child(self.render_error_banner(cx))
                        .child(self.render_terminal(cx))
                        // Tracked: clicking here must move real keyboard
                        // focus, or typed keys never reach `muse`. Drag
                        // highlights terminal text (copy-on-select);
                        // Cmd+C copies, Cmd/Ctrl+V pastes.
                        .track_focus(&self.term_focus.clone().unwrap())
                        .id(ElementId::Name("terminal-pane".into()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, window, _cx| {
                                this.focus_term(window);
                                this.begin_selection(ev.position);
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, _cx| {
                            this.update_selection(ev.position);
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseUpEvent, _window, cx| {
                                this.end_selection(ev.position, cx);
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseUpEvent, _window, cx| {
                                this.end_selection(ev.position, cx);
                            }),
                        )
                        .on_click(cx.listener(|this, _ev, window, _cx| {
                            this.focus_term(window);
                        })),
                ),
            )
            .child(
                div()
                    .h(px(STATUS_HEIGHT))
                    .px_2()
                    .bg(rgb(0x1e1e2e))
                    .text_color(rgb(0x888888))
                    .text_sm()
                    .truncate()
                    .child(self.status_text_for_width(viewport_w))
                    .id(ElementId::Name("status-bar".into()))
                    .on_click(cx.listener(|this, _ev, window, _cx| {
                        this.focus_list(window);
                    })),
            )
    }
}

/// Window options for the main window. The minimum size is derived from
/// the PTY floors (see [`MIN_WINDOW_WIDTH`]/[`MIN_WINDOW_HEIGHT`]) so the
/// OS never shrinks the window past what the terminal grid can display.
pub fn window_options() -> WindowOptions {
    WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(SharedString::from("Agent Manager")),
            ..Default::default()
        }),
        window_min_size: Some(Size {
            width: px(MIN_WINDOW_WIDTH),
            height: px(MIN_WINDOW_HEIGHT),
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::terminal::{screen_rows, Rgb8};
    use super::super::terminal_pane::layout_text;
    use super::*;

    #[test]
    fn layout_text_partition_satisfies_with_runs() {
        // Regression test for the "new session" crash: gpui validates that
        // run lengths partition the text byte-exactly and panics otherwise.
        // This calls the real constructor, so it panics here first.
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[2J\x1b[1;1Htop \x1b[31mred\x1b[0m \xc3\xa9\xe2\x9d\xaf");
        let rows = screen_rows(parser.screen(), Some((0, 0)), Rgb8(200, 200, 200));
        let (full, runs) = layout_text(&rows);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len(), "runs must cover every byte");
        assert!(runs.iter().all(|r| r.len > 0), "no empty runs");
        let _ = gpui::StyledText::new(full).with_runs(runs);
        // Empty screen still partitions (single covered space per row).
        let mut empty = vt100::Parser::new(24, 80, 0);
        empty.process(b"");
        let rows = screen_rows(empty.screen(), None, Rgb8(0, 0, 0));
        let (full, runs) = layout_text(&rows);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len());
        let _ = gpui::StyledText::new(full).with_runs(runs);
    }

    #[test]
    fn render_terminal_with_live_pty_does_not_panic() {
        // End-to-end of the "new session" crash path: a real child writes
        // colored output, and render_terminal builds the gpui element.
        // Pre-fix this panicked inside StyledText::with_runs.
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = crate::embedded::EmbeddedPty::spawn(
            "printf",
            &["\\x1b[2J\\x1b[1;1Hhi \\x1b[31mred\\n\"".to_string()],
            80,
            24,
        )
        .unwrap();
        view.runs.insert(id.clone(), Run::new(pty));
        view.runs.get_mut(&id).unwrap().last_output = std::time::Instant::now();
        for _ in 0..50 {
            view.refresh();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let live = view.active_view().expect("live pty has a view");
        let _ = view.render_live_terminal(live);
    }

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
    fn narrow_viewport_collapses_sidebar_and_frees_pty_width() {
        // Issue #6: below ~700px the 264px sidebar collapses so the
        // terminal pane gets the full width.
        assert!(!sidebar_visible_for_width(699.0));
        assert!(sidebar_visible_for_width(700.0));
        assert!(sidebar_visible_for_width(1280.0));
        assert_eq!(effective_sidebar_width(699.0), 0.0);
        assert_eq!(effective_sidebar_width(800.0), super::LEFT_WIDTH);
        // A 600px narrow window gives the PTY the full 600px (75 cols at
        // 8px) instead of 600-264=336px (42 cols) beside the sidebar.
        let (collapsed_cols, _) = super::pty_grid_for(600.0, 400.0, 8.0, 18.0);
        let (beside_cols, _) = super::pty_grid_for(600.0 - super::LEFT_WIDTH, 400.0, 8.0, 18.0);
        assert_eq!(collapsed_cols, 75);
        assert_eq!(beside_cols, 42);
        assert!(collapsed_cols > beside_cols);
    }

    #[test]
    fn pty_grid_floors_and_ceilings_match_window_minimum() {
        // Degenerate sizes still report a usable grid (cols>=20/rows>=10):
        // the 200x120px floors divide to 25x6 at 8x18 metrics, and the row
        // clamp lifts 6 to the MIN_ROWS floor of 10.
        assert_eq!(super::pty_grid_for(0.0, 0.0, 8.0, 18.0), (25, 10));
        assert_eq!(super::pty_grid_for(-50.0, -50.0, 8.0, 18.0), (25, 10));
        // Huge windows clamp instead of overflowing the u16 grid.
        assert_eq!(
            super::pty_grid_for(100_000.0, 100_000.0, 8.0, 18.0),
            (400, 200)
        );
        // Ordinary sizes divide exactly.
        assert_eq!(super::pty_grid_for(800.0, 360.0, 8.0, 18.0), (100, 20));
    }

    #[test]
    fn window_min_size_matches_pty_floors() {
        // Issue #6: the OS minimum must fit the PTY floors, not clip them.
        let min = super::window_options()
            .window_min_size
            .expect("main window sets a minimum size");
        assert_eq!(f32::from(min.width), super::MIN_WINDOW_WIDTH);
        assert_eq!(f32::from(min.height), super::MIN_WINDOW_HEIGHT);
        // At the minimum size the terminal pane still fits a full
        // MIN_COLS x MIN_ROWS grid at fallback metrics.
        let (cols, rows) = super::pty_grid_for(
            f32::from(min.width) - super::LEFT_WIDTH,
            f32::from(min.height) - super::STATUS_HEIGHT - super::HEADER_HEIGHT,
            super::FALLBACK_CHAR_W,
            super::FALLBACK_LINE_H,
        );
        assert!(cols >= super::MIN_COLS, "min width fits {cols} cols");
        assert!(rows >= super::MIN_ROWS, "min height fits {rows} rows");
    }

    #[test]
    fn narrow_status_hints_stay_compact_but_complete() {
        // Issue #6: long key-hint lines break below ~700px; narrow widths
        // get compact hints that still name every essential key.
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        view.app.focus_nav();
        let full = view.status_text_for_width(1280.0);
        let narrow = view.status_text_for_width(600.0);
        assert_eq!(full, view.status_text(), "wide hints are unchanged");
        assert!(
            narrow.len() < full.len(),
            "narrow hints compact: {narrow:?} vs {full:?}"
        );
        for key in ["n:", "j/k", "o/Enter", "Tab", "x:", "q:"] {
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
