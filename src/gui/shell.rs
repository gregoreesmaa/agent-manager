//! gpui shell: runs panel, faithful terminal pane, status bar.
//!
//! Thin by design (regression shield): all parsing, key encoding, and style
//! mapping live in framework-free modules ([`super::terminal`],
//! [`super::keys`], [`crate::app`]); this view only lays out gpui elements,
//! forwards events, and pumps PTYs. Shell logic that needs no window (pump,
//! refresh, spawn) is covered by plain-struct headless tests below — no
//! gpui test harness required.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    div, font, px, rgb, AnyElement, App as GpuiApp, Bounds, ClipboardItem, Context, ElementId,
    FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString, Size,
    StatefulInteractiveElement, Styled, StyledText, TextRun, UnderlineStyle, Window, WindowOptions,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    Sizable as _,
};

use crate::app::{section_title, status_sections, App, Status};
use crate::embedded::{EmbeddedPty, LiveView};
use crate::parsers::github::extract_pr_links;

use super::keys::{keystroke_to_pty, KeyPress};
use super::terminal::{
    point_to_cell, screen_rows, selection_rows, selection_text, to_hsla, CellPos, Rgb8,
};

/// Left runs panel width in pixels.
const LEFT_WIDTH: f32 = 264.0;
/// Status bar height in pixels.
const STATUS_HEIGHT: f32 = 28.0;
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
const TERM_FONT_SIZE: f32 = 13.0;
/// Pump cadence: the background task polls PTYs at 20 Hz; actual
/// repaints are dirty-gated (see `tick`), so idle costs ~zero.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(50);
/// Selection highlight behind terminal text (classic selection blue).
const SELECTION_BG: u32 = 0x264f78;
/// Default terminal foreground when the child requests the default color.
const DEFAULT_FG: Rgb8 = Rgb8(212, 212, 212);
/// Caret color for the emulated cursor cell.
const CURSOR_BG: Rgb8 = Rgb8(180, 180, 180);

/// Outcome of a nav-focus keypress: state changes apply immediately,
/// window/clipboard effects are applied by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavAction {
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

pub struct ShellView {
    app: App,
    ptys: HashMap<String, EmbeddedPty>,
    last_output: HashMap<String, Instant>,
    pending_inputs: HashMap<String, String>,
    /// Cached attention bit per run: unchanged screens skip the text
    /// scans entirely (see `refresh`). Entries die with their run.
    attention_cache: HashMap<String, bool>,
    /// Two-step quit arming: the first `q` with dirty runs only arms and
    /// hints; the second quits. Any other key disarms.
    quit_armed: bool,
    list_focus: Option<FocusHandle>,
    term_focus: Option<FocusHandle>,
    cols: u16,
    rows: u16,
    focused_once: bool,
    /// Window-space bounds of the terminal text, captured each frame so a
    /// mouse drag maps back to terminal cells for highlighting.
    term_text_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// Last measured monospace cell size, for drag→cell mapping and for
    /// painting the selection highlight behind the text.
    char_w: f32,
    line_h: f32,
    /// Mouse-drag selection in terminal cells (anchor, cursor). `None` while
    /// no drag is in progress / no selection exists.
    sel_anchor: Option<CellPos>,
    sel_active: Option<CellPos>,
    selecting: bool,
    /// Keyboard-focused parsed link: index into the selected run's
    /// `pr_links`. `None` means no link is focused (Enter focuses the
    /// terminal). Cleared whenever the run selection moves.
    link_cursor: Option<usize>,
}

impl ShellView {
    pub fn new() -> Self {
        Self {
            app: App::new(vec![]),
            ptys: HashMap::new(),
            last_output: HashMap::new(),
            pending_inputs: HashMap::new(),
            attention_cache: HashMap::new(),
            quit_armed: false,
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
            link_cursor: None,
        }
    }

    fn active_id(&self) -> Option<String> {
        self.app.selected_session().map(|s| s.id.clone())
    }

    fn active_view(&self) -> Option<LiveView<'_>> {
        self.active_id()
            .as_ref()
            .and_then(|id| self.ptys.get(id))
            .map(|pty| pty.view())
    }

    /// Start the queued `muse` for the newly created run. Returns true when
    /// a spawn was consumed: success adds a live view, failure sticks an
    /// error banner — either way the screen changed and needs a repaint.
    fn spawn_queued(&mut self) -> bool {
        if let Some(kind) = self.app.take_pending_spawn() {
            let run_id = self.active_id().unwrap_or_default();
            match EmbeddedPty::spawn_kind(&kind, self.cols, self.rows) {
                Ok(pty) => {
                    self.ptys.insert(run_id.clone(), pty);
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

    /// Success bookkeeping for a fresh spawn: track output recency and
    /// clear any sticky error (errors stay until dismissed/next success).
    fn note_spawn_success(&mut self, run_id: &str) {
        self.last_output.insert(run_id.to_string(), Instant::now());
        self.app.clear_error();
    }

    /// Retry is offered when a spawn actually failed and the selected run
    /// still owns no live PTY. Gating on the recorded failure (not just a
    /// missing PTY) keeps a fast-typed `r` reaching a still-starting child
    /// instead of queueing a duplicate spawn.
    fn can_retry(&self) -> bool {
        if self.app.error_text().is_none() {
            return false;
        }
        match self.active_id() {
            Some(id) => !self.ptys.contains_key(&id),
            None => false,
        }
    }

    /// Retry action for a failed spawn: re-queue a fresh `muse` for the
    /// selected run (same id, no new entry). Never touches a live PTY.
    fn retry_spawn(&mut self) {
        if self.can_retry() {
            self.app.retry_spawn();
        }
    }

    /// Restart is offered exactly when the selected run's child exited:
    /// the most common lifecycle event, previously a dead end.
    fn can_restart(&self) -> bool {
        self.active_view().is_some_and(|v| v.exited)
    }

    /// Restart the selected ended run: drop the dead PTY (reaping the
    /// child), forget its output recency and pending input, and queue a
    /// fresh spawn on the same run id. Title and accumulated links are
    /// kept — restart resumes the run's story, it does not archive it.
    /// No-op unless the active child exited (live runs are never killed
    /// by accident; per-run close is a separate explicit action).
    fn restart_run(&mut self) {
        if !self.can_restart() {
            return;
        }
        if let Some(id) = self.active_id() {
            self.ptys.remove(&id);
            self.last_output.remove(&id);
            self.pending_inputs.remove(&id);
            self.attention_cache.remove(&id);
            self.clear_selection();
            self.app.retry_spawn();
        }
    }

    /// Close (kill) the selected run: drop its live PTY — `Drop` kills
    /// and reaps the child so no zombie survives — and remove its entry.
    /// Immediate and single-step; quitting the whole app is what asks.
    /// No-op when no run is selected.
    fn close_run(&mut self) {
        if let Some(id) = self.active_id() {
            self.ptys.remove(&id);
            self.last_output.remove(&id);
            self.pending_inputs.remove(&id);
            self.attention_cache.remove(&id);
            self.clear_selection();
            if let Some(title) = self.app.remove_session(&id) {
                self.app.set_status(format!("closed '{title}'"));
            }
        }
    }

    /// True when quitting must confirm first: any Working/Attention run
    /// or any live (non-exited) PTY. Empty/idle shells quit instantly.
    fn confirm_quit_required(&self) -> bool {
        self.app.needs_quit_confirm() || self.ptys.values().any(|pty| !pty.view().exited)
    }

    /// Two-step quit: returns true when the app should exit now. The
    /// first call with dirty runs only arms (with a hint); the second
    /// quits. Safe states quit on the first call and never arm.
    fn request_quit(&mut self) -> bool {
        if !self.confirm_quit_required() {
            return true;
        }
        if self.quit_armed {
            return true;
        }
        self.quit_armed = true;
        false
    }

    /// One pump iteration for the background task. Returns true when the
    /// pump observed anything visible — fresh output, a newly exited child,
    /// a consumed spawn, a status flip, a new PR link, or a moved row — so
    /// the caller repaints only then and idle costs ~zero instead of a full
    /// refresh + repaint at 20 Hz. Selection/focus moves are event-driven
    /// (they repaint directly), so the pump only tracks PTY-derived change.
    pub fn tick(&mut self) -> bool {
        self.refresh()
    }

    /// Pump every run, refresh each entry from its live screen (attention
    /// markers, working/idle by recency, PR links), re-sort pinned.
    /// Returns true when anything visible changed (see [`ShellView::tick`]).
    /// Text scans (attention regex + link extraction) run only for runs
    /// whose PTY delivered bytes or changed exit state since the last tick;
    /// unchanged screens reuse the cached attention bit, so an idle tick
    /// costs no screen allocs at all. Re-sorting runs on the same gate: row
    /// order only depends on status, so an unchanged pump leaves the order
    /// (and the selection index) untouched instead of re-sorting every tick.
    fn refresh(&mut self) -> bool {
        let spawned = self.spawn_queued();
        let mut fresh_any = false;
        let mut rescanned: Vec<String> = Vec::new();
        for (id, pty) in self.ptys.iter_mut() {
            let exited_before = pty.view().exited;
            if pty.pump() {
                fresh_any = true;
                self.last_output.insert(id.clone(), Instant::now());
                rescanned.push(id.clone());
            } else if pty.view().exited != exited_before {
                // No new bytes, but the child just exited: the run's
                // status and links are stale, so rescan it as dirt.
                fresh_any = true;
                rescanned.push(id.clone());
            }
        }
        let now = Instant::now();
        let ids: Vec<String> = self.ptys.keys().cloned().collect();
        let mut changed = spawned || fresh_any;
        for id in ids {
            // Scope the PTY borrow: the merge below touches other fields.
            let (attention, exited, fresh_links) = {
                let pty = &self.ptys[&id];
                let view = pty.view();
                if rescanned.contains(&id) {
                    let text = view.screen.contents();
                    let attention = crate::app::needs_attention(&text);
                    let pr = extract_pr_links(&text);
                    let related = crate::parsers::related_links(&text);
                    (attention, view.exited, Some((pr, related)))
                } else {
                    (
                        self.attention_cache.get(&id).copied().unwrap_or(false),
                        view.exited,
                        None,
                    )
                }
            };
            if rescanned.contains(&id) {
                self.attention_cache.insert(id.clone(), attention);
            }
            // One classifier for live and historic runs alike (owned by
            // `app`): attention markers win, then exit, then recency.
            let age = self.last_output.get(&id).map(|at| now.duration_since(*at));
            let status = crate::app::classify_with_attention(attention, age, exited);
            // Accumulate PR links in first-seen order: the visible screen
            // is only a viewport (vt100 `contents()` shows the live grid,
            // not full scrollback), so replacing would drop links that
            // scrolled off. Merging keeps every PR URL ever seen per run.
            if let Some(s) = self.app.sessions.iter_mut().find(|s| s.id == id) {
                if s.status != status {
                    s.status = status;
                    changed = true;
                }
                // Cap-not-drop merge (storage cap + truncation flag live
                // in `push_links`); the panel folds extras behind N more.
                // Gated on actual growth so a rescan with no new links
                // stays clean and never repaints.
                if let Some((pr, related)) = fresh_links {
                    let before = (s.pr_links.len(), s.related_links.len(), s.links_truncated);
                    s.push_links(pr, related);
                    if (s.pr_links.len(), s.related_links.len(), s.links_truncated) != before {
                        changed = true;
                    }
                }
            }
        }
        if changed {
            // Row order derives from status alone (last_active never moves
            // here), so a changed pump is exactly when rows could have
            // moved; a clean pump leaves order and selection index alone.
            self.app.resort_keep_selection();
        }
        changed
    }

    /// Forward one gpui key event to the active PTY (Terminal focus),
    /// tracking the input line so the first submitted prompt titles the run.
    fn forward_key(&mut self, key: &str, key_char: Option<&str>, ctrl: bool, alt: bool) {
        let Some(id) = self.active_id() else {
            return;
        };
        match key {
            "enter" => {
                if let Some(line) = self.pending_inputs.remove(&id) {
                    self.app.note_submitted_prompt(&id, &line);
                }
            }
            "backspace" => {
                if let Some(buf) = self.pending_inputs.get_mut(&id) {
                    buf.pop();
                }
            }
            _ => {
                let text = key_char.filter(|c| !c.is_empty()).unwrap_or(key);
                if text.chars().count() == 1 && !ctrl {
                    self.pending_inputs
                        .entry(id.clone())
                        .or_default()
                        .push_str(text);
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
            if let Some(pty) = self.ptys.get_mut(&id) {
                if let Err(e) = pty.write_input(&bytes) {
                    self.app.set_error(e.to_string());
                }
            }
        }
    }

    /// Normalized, non-empty mouse selection in terminal cells, if any.
    fn selection_pair(&self) -> Option<(CellPos, CellPos)> {
        let (a, b) = (self.sel_anchor?, self.sel_active?);
        let (start, end) = super::terminal::normalize_selection(a, b);
        if start == end {
            None
        } else {
            Some((start, end))
        }
    }

    /// Text covered by the mouse selection, via the emulator's own
    /// `contents_between` (accurate for wrapped/wide cells).
    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_pair()?;
        let view = self.active_view()?;
        let text = selection_text(view.screen, start, end);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.sel_active = None;
        self.selecting = false;
    }

    /// Map a window-space mouse point to a terminal cell using the captured
    /// text bounds plus the measured cell size and live grid size.
    fn mouse_cell(&self, pos: Point<Pixels>) -> Option<CellPos> {
        let bounds = (*self.term_text_bounds.borrow())?;
        let view = self.active_view()?;
        let (rows, cols) = view.screen.size();
        Some(point_to_cell(
            (f32::from(pos.x), f32::from(pos.y)),
            (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
            self.char_w,
            self.line_h,
            cols,
            rows,
        ))
    }

    fn begin_selection(&mut self, pos: Point<Pixels>) {
        if let Some(cell) = self.mouse_cell(pos) {
            self.sel_anchor = Some(cell);
            self.sel_active = Some(cell);
            self.selecting = true;
        } else {
            self.clear_selection();
        }
    }

    fn update_selection(&mut self, pos: Point<Pixels>) {
        if !self.selecting {
            return;
        }
        if let Some(cell) = self.mouse_cell(pos) {
            self.sel_active = Some(cell);
        }
    }

    /// Finish a drag: copy-on-select when the drag covered text, mirroring
    /// terminal copy-on-select behavior.
    fn end_selection(&mut self, pos: Point<Pixels>, cx: &mut GpuiApp) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        if let Some(cell) = self.mouse_cell(pos) {
            self.sel_active = Some(cell);
        }
        if let Some(text) = self.selected_text() {
            let chars = text.chars().count();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.app
                .set_status(format!("copied selection ({chars} chars)"));
        } else {
            self.clear_selection();
        }
    }

    /// Copy the mouse selection when one exists, else the whole active
    /// screen, to the system clipboard.
    fn copy_screen(&mut self, cx: &mut GpuiApp) {
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
    fn paste_clipboard(&mut self, cx: &mut GpuiApp) {
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
        if let Some(pty) = self.ptys.get_mut(&id) {
            if let Err(e) = pty.write_input(text.as_bytes()) {
                self.app.set_error(e.to_string());
            } else {
                let chars = text.chars().count();
                self.app.set_status(format!("pasted {chars} chars"));
            }
        }
    }

    /// Nav-focus key dispatch, pure state (headlessly testable). The caller
    /// applies window focus and clipboard effects for the returned action.
    /// `muse` captures keys if and only if focus is Terminal — nav keys
    /// never reach the PTY.
    /// URL of the keyboard-focused parsed link, if the selected run has
    /// one at [`Self::link_cursor`].
    fn focused_link_url(&self) -> Option<String> {
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
    fn cycle_link_focus(&mut self) {
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

    fn nav_action(&mut self, key: &str, ctrl: bool) -> NavAction {
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

    fn focus_term(&mut self, window: &mut Window) {
        self.app.focus_terminal();
        if let Some(h) = &self.term_focus {
            window.focus(h);
        }
    }

    fn focus_list(&mut self, window: &mut Window) {
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

    /// Resize the active PTY to the central pane, measured in monospace cells.
    /// Narrow viewports (<700px) collapse the sidebar, so the pane (and the
    /// PTY) use the full window width instead of squeezing beside 264px.
    fn fit_pty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        let avail_w =
            f32::from(viewport.width) - effective_sidebar_width(f32::from(viewport.width));
        let avail_h = f32::from(viewport.height) - STATUS_HEIGHT;
        let (char_w, line_h) = mono_metrics(cx);
        self.char_w = char_w;
        self.line_h = line_h;
        let (cols, rows) = pty_grid_for(avail_w, avail_h, char_w, line_h);
        if (cols, rows) != (self.cols, self.rows) {
            self.cols = cols;
            self.rows = rows;
            if let Some(id) = self.active_id() {
                if let Some(pty) = self.ptys.get_mut(&id) {
                    pty.resize(cols, rows);
                }
            }
        }
    }

    /// Sessions panel as library chrome: a [`Sidebar`] with one
    /// [`SidebarGroup`] per status section (Needs input / Idle / Active),
    /// session rows as [`SidebarMenuItem`]s (active = selected), and every
    /// accumulated PR link as a child item that copies its URL on click.
    /// The sidebar scrolls internally, so long link lists never push
    /// sessions off-panel. The terminal pane stays hand-rolled gpui.
    fn render_runs(&self, cx: &mut Context<Self>) -> AnyElement {
        // Header with a real button: sessions start here, not at a key hint.
        let mut sidebar = Sidebar::left().w(px(LEFT_WIDTH)).header(
            SidebarHeader::new().child("Sessions".to_string()).child(
                Button::new(ElementId::Name("new-run-btn".into()))
                    .label("+ New")
                    .primary()
                    .small()
                    .on_click(cx.listener(|this, _ev, window, _cx| {
                        this.app.start_new_session();
                        this.clear_selection();
                        this.focus_term(window);
                    })),
            ),
        );
        if self.app.sessions.is_empty() {
            return sidebar
                .footer(
                    div()
                        .text_color(rgb(0x888888))
                        .text_xs()
                        .child("No sessions yet.".to_string()),
                )
                .into_any_element();
        }
        for (status, indices) in status_sections(&self.app.sessions) {
            let marker = match status {
                Status::Attention => "!",
                Status::Idle => "·",
                Status::Working => ">",
            };
            let items: Vec<SidebarMenuItem> = indices
                .into_iter()
                .map(|i| {
                    let s = &self.app.sessions[i];
                    let row_marker = match s.status {
                        Status::Attention => "!",
                        Status::Idle => " ",
                        Status::Working => ">",
                    };
                    let row_id = s.id.clone();
                    // Parsed links as child items: click copies the full
                    // URL. Display-capped: the first MAX_VISIBLE_LINKS
                    // rows stay live and the rest fold behind an N more
                    // disclosure; the full lists stay retained (and
                    // copyable via yank) in the session. `o`/Enter does
                    // the same from the keyboard when a PR row holds link
                    // focus (highlighted via `active`).
                    let short_of = |link: &str| {
                        link.trim_start_matches("https://")
                            .trim_start_matches("http://")
                            .trim_start_matches("github.com/")
                            .to_string()
                    };
                    let mut links: Vec<SidebarMenuItem> = Vec::new();
                    let (shown_prs, hidden_prs) = crate::app::visible_links(&s.pr_links);
                    for (li, link) in shown_prs.iter().enumerate() {
                        let short = short_of(link);
                        let url = link.clone();
                        let focused = i == self.app.selected && self.link_cursor == Some(li);
                        links.push(SidebarMenuItem::new(short).active(focused).on_click(
                            cx.listener(move |this, _ev, _window, cx| {
                                this.link_cursor = Some(li);
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                this.app.set_status("copied PR link");
                            }),
                        ));
                    }
                    let (shown_rel, hidden_rel) = crate::app::visible_links(&s.related_links);
                    for link in shown_rel {
                        let short = short_of(link);
                        let url = link.clone();
                        links.push(SidebarMenuItem::new(short).on_click(cx.listener(
                            move |this, _ev, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                this.app.set_status("copied link");
                            },
                        )));
                    }
                    let hidden = hidden_prs + hidden_rel;
                    if hidden > 0 {
                        links.push(SidebarMenuItem::new(format!("… {hidden} more")));
                    }
                    if s.links_truncated {
                        links.push(SidebarMenuItem::new("(capped at 50 per list)".to_string()));
                    }
                    SidebarMenuItem::new(format!("{row_marker} {}", s.title))
                        .active(i == self.app.selected)
                        .default_open(true)
                        .on_click(cx.listener(move |this, _ev, window, _cx| {
                            if let Some(pos) = this.app.sessions.iter().position(|s| s.id == row_id)
                            {
                                this.app.selected = pos;
                            }
                            this.clear_selection();
                            this.link_cursor = None;
                            this.focus_list(window);
                        }))
                        .children(links)
                })
                .collect();
            let group = SidebarGroup::new(format!(
                "{marker} {} ({})",
                section_title(status),
                items.len()
            ))
            .child(SidebarMenu::new().children(items));
            sidebar = sidebar.child(group);
        }
        sidebar.into_any_element()
    }

    /// First-run orientation copy: icon + message + error flag so the
    /// empty state (`No session yet`) and the failure state (`spawn
    /// failed`) differ by shape and color, not text alone.
    fn empty_pane_copy(&self) -> (&'static str, String, bool) {
        match self.app.error_text() {
            Some(e) => ("✕", format!("spawn failed: {e}"), true),
            None => (
                "○",
                "No session yet. Press n to start a new muse.".to_string(),
                false,
            ),
        }
    }

    /// Empty terminal pane: icon-split status line plus a clickable
    /// `+ New` CTA (keyboard parity: `n` still works) so first run does
    /// not depend on discovering the key hint.
    fn render_empty_pane(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (icon, msg, is_error) = self.empty_pane_copy();
        let color = if is_error {
            rgb(0xff9999)
        } else {
            rgb(0x888888)
        };
        div().flex_1().h_full().p_4().child(
            div()
                .flex()
                .flex_col()
                .child(div().text_color(color).child(format!("{icon} {msg}")))
                .child(
                    div().pt_2().child(
                        Button::new(ElementId::Name("empty-new-run-btn".into()))
                            .label("+ New (n)")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _ev, window, _cx| {
                                this.app.start_new_session();
                                this.clear_selection();
                                this.focus_term(window);
                            })),
                    ),
                ),
        )
    }

    fn render_terminal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(view) = self.active_view() else {
            return self.render_empty_pane(cx);
        };
        self.render_live_terminal(view)
    }

    /// Live terminal pane. Takes no window context so headless tests can
    /// build the element without a gpui harness.
    fn render_live_terminal(&self, view: LiveView<'_>) -> gpui::Div {
        let cursor = if view.exited || view.screen.hide_cursor() {
            None
        } else {
            Some(view.screen.cursor_position())
        };
        let rows = screen_rows(view.screen, cursor, CURSOR_BG);
        let (full, runs) = layout_text(&rows);
        let text = StyledText::new(full).with_runs(runs);
        // Mouse selection highlight: cell rectangles behind the text. The
        // inner wrapper has no padding, so overlay origin == text origin and
        // no padding constant is needed.
        let (_, cols) = view.screen.size();
        let spans = match self.selection_pair() {
            Some((s, e)) => selection_rows(s, e, cols),
            None => Vec::new(),
        };
        let slot = Rc::clone(&self.term_text_bounds);
        let mut inner = div().relative().h_full().w_full();
        for (row, sc, ec) in spans {
            if ec <= sc {
                continue;
            }
            inner = inner.child(
                div()
                    .absolute()
                    .left(px(sc as f32 * self.char_w))
                    .top(px(row as f32 * self.line_h))
                    .w(px((ec - sc) as f32 * self.char_w))
                    .h(px(self.line_h))
                    .bg(rgb(SELECTION_BG)),
            );
        }
        inner = inner.child(
            div()
                .child(text)
                .on_children_prepainted(move |bounds, _, _| {
                    if let Some(b) = bounds.first() {
                        *slot.borrow_mut() = Some(*b);
                    }
                }),
        );
        div()
            .flex_1()
            .h_full()
            .bg(rgb(0x11111b))
            .p_2()
            .font_family("Menlo")
            .text_size(px(TERM_FONT_SIZE))
            .child(inner)
    }

    /// Sticky-error banner above the terminal pane: spawn and PTY-write
    /// failures stay visible here (mirroring the status bar) until
    /// dismissed or superseded by a success. Retry appears only when the
    /// selected run owns no live PTY; Dismiss (`d`) always does.
    /// Empty (zero-size) when no error is sticky.
    fn render_error_banner(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(err) = self.app.error_text().map(str::to_string) else {
            return div();
        };
        let retry = self.can_retry();
        let mut row = div()
            .flex()
            .flex_row()
            .px_2()
            .py_1()
            .bg(rgb(0x3a1d1d))
            .text_color(rgb(0xff9999))
            .text_sm()
            .child(err);
        if retry {
            row = row.child(
                Button::new(ElementId::Name("retry-spawn-btn".into()))
                    .label("Retry (r)")
                    .primary()
                    .small()
                    .on_click(cx.listener(|this, _ev, window, _cx| {
                        this.retry_spawn();
                        this.focus_term(window);
                    })),
            );
        }
        row.child(
            Button::new(ElementId::Name("dismiss-error-btn".into()))
                .label("Dismiss (d)")
                .small()
                .on_click(cx.listener(|this, _ev, _window, _cx| {
                    this.app.clear_error();
                })),
        )
    }

    fn status_text(&self) -> String {
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

/// Flatten screen rows into one string plus gpui text runs. Every byte of
/// the string belongs to exactly one non-empty run — gpui validates this
/// partition and panics otherwise (crashed the first launch).
fn layout_text(rows: &[Vec<super::terminal::TermSpan>]) -> (String, Vec<TextRun>) {
    let mono = font("Menlo");
    let plain = || TextRun {
        len: 0,
        font: mono.clone(),
        color: to_hsla(DEFAULT_FG),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut full = String::new();
    let mut runs = Vec::new();
    let push_text = |full: &mut String, runs: &mut Vec<TextRun>, text: &str, run: TextRun| {
        let start = full.len();
        full.push_str(text);
        let mut run = run;
        run.len = full.len() - start;
        if run.len > 0 {
            runs.push(run);
        }
    };
    for (ri, row) in rows.iter().enumerate() {
        if ri > 0 {
            push_text(&mut full, &mut runs, "\n", plain());
        }
        if row.is_empty() {
            push_text(&mut full, &mut runs, " ", plain());
        }
        for span in row {
            let fg = span
                .style
                .fg
                .map(to_hsla)
                .unwrap_or_else(|| to_hsla(DEFAULT_FG));
            let bg = span.style.bg.map(to_hsla);
            push_text(
                &mut full,
                &mut runs,
                &span.text,
                TextRun {
                    len: 0,
                    font: if span.style.bold {
                        mono.clone().bold()
                    } else {
                        mono.clone()
                    },
                    color: fg,
                    background_color: bg,
                    underline: span.style.underline.then_some(UnderlineStyle::default()),
                    strikethrough: None,
                },
            );
        }
    }
    (full, runs)
}

/// Monospace cell metrics for PTY sizing, with sane fallbacks.
fn mono_metrics(cx: &mut Context<ShellView>) -> (f32, f32) {
    let size = px(TERM_FONT_SIZE);
    let system = cx.text_system();
    let fid = system.resolve_font(&font("Menlo"));
    let char_w = system
        .advance(fid, size, ' ')
        .map(|s| f32::from(s.width))
        .unwrap_or(8.0);
    let line_h = f32::from(system.ascent(fid, size)) + f32::from(system.descent(fid, size));
    let line_h = if line_h > 0.0 { line_h } else { 18.0 };
    (char_w.max(4.0), line_h.max(8.0))
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
mod headless_tests {
    use super::*;

    /// Plain-struct headless coverage: real PTYs, no window needed.
    fn view_with_echo() -> ShellView {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn("echo", &["hello-gui".to_string()], 80, 24).unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id, Instant::now());
        view
    }

    #[test]
    fn refresh_tracks_live_output_and_exit() {
        let mut view = view_with_echo();
        view.refresh();
        let id = view.active_id().unwrap();
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert!(s.pr_links.is_empty());
        // echo exits on its own; poll until the reaped exit flips it idle.
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if s.status == Status::Idle {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.status, Status::Idle);
    }

    #[test]
    fn tick_gates_repaint_on_dirtiness() {
        use std::time::Duration;

        // Clean: a live run with no output never requests a repaint, so
        // the 20 Hz pump idles instead of burning a refresh + repaint.
        let mut idle = ShellView::new();
        idle.app.start_new_session();
        let _ = idle.app.take_pending_spawn();
        let id = idle.active_id().unwrap();
        let pty = EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        idle.ptys.insert(id.clone(), pty);
        idle.last_output.insert(id, Instant::now());
        for _ in 0..5 {
            assert!(!idle.tick(), "idle pump must stay clean (no repaint)");
        }

        // Dirty: fresh PTY output requests a repaint.
        let mut live = ShellView::new();
        live.app.start_new_session();
        let _ = live.app.take_pending_spawn();
        let id = live.active_id().unwrap();
        let pty = EmbeddedPty::spawn("printf", &["hello-dirty\\n".to_string()], 80, 24).unwrap();
        live.ptys.insert(id.clone(), pty);
        live.last_output.insert(id.clone(), Instant::now());
        let mut saw_dirty = false;
        for _ in 0..100 {
            if live.tick() {
                saw_dirty = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_dirty, "fresh PTY output must mark the pump dirty");

        // Settled: once output is drained and the run is idle, the pump
        // goes clean again (no per-tick repaint at 20 Hz).
        for _ in 0..100 {
            live.tick();
            let s = live.app.sessions.iter().find(|s| s.id == id).unwrap();
            if s.status == Status::Idle {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let s = live.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.status, Status::Idle);
        assert!(!live.tick(), "settled idle pump must stay clean");
        assert!(!live.tick(), "settled idle pump must stay clean");

        // Dirty without output: a child that exits silently still flips
        // the pump dirty once (the ended state needs its repaint).
        let mut quick = ShellView::new();
        quick.app.start_new_session();
        let _ = quick.app.take_pending_spawn();
        let qid = quick.active_id().unwrap();
        let pty = EmbeddedPty::spawn("true", &[], 80, 24).unwrap();
        quick.ptys.insert(qid.clone(), pty);
        quick.last_output.insert(qid, Instant::now());
        let mut saw_exit_dirty = false;
        for _ in 0..100 {
            if quick.tick() {
                saw_exit_dirty = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_exit_dirty, "silent exit must mark the pump dirty");
    }

    #[test]
    fn refresh_extracts_pr_links_from_screen() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn(
            "printf",
            &["see https://github.com/acme/app/pull/42\\n".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id.clone(), Instant::now());
        // Pump until the printf output lands, then refresh reads the screen.
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if !s.pr_links.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            s.pr_links,
            vec!["https://github.com/acme/app/pull/42".to_string()]
        );
    }

    #[test]
    fn attention_status_comes_from_screen_text() {
        let mut view = view_with_echo();
        view.refresh();
        // Fake an approval prompt on the screen via printf run.
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn(
            "printf",
            &["Waiting for your approval to proceed\\n".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if s.status == Status::Attention {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.status, Status::Attention);
    }

    #[test]
    fn nav_keys_never_reach_the_pty_only_terminal_focus_forwards() {
        let mut view = ShellView::new();
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
        let mut view = ShellView::new();
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
        let mut view = ShellView::new();
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

    #[test]
    fn retry_requeues_failed_spawn_and_never_touches_live_pty() {
        let mut view = ShellView::new();
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
        let id = view.active_id().unwrap();
        let _ = view.app.take_pending_spawn();
        let pty = EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.ptys.insert(id, pty);
        assert!(!view.can_retry());
        assert_eq!(view.nav_action("r", false), NavAction::None);
    }

    #[test]
    fn write_failure_with_live_pty_stays_sticky_in_status_bar() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.ptys.insert(id, pty);
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
        let mut view = ShellView::new();
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
    fn restart_is_offered_only_for_exited_runs_and_keeps_the_run() {
        use std::time::Duration;

        let mut view = ShellView::new();
        assert!(!view.can_restart());
        assert_eq!(view.nav_action("r", false), NavAction::None);
        view.restart_run();
        assert!(view.app.take_pending_spawn().is_none());

        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        // Live child: restart is never offered (no accidental kills).
        let live = EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.ptys.insert(id.clone(), live);
        view.last_output.insert(id.clone(), Instant::now());
        assert!(!view.can_restart());
        assert_eq!(view.nav_action("r", false), NavAction::None);
        view.restart_run();
        assert!(view.ptys.contains_key(&id));
        assert!(view.app.take_pending_spawn().is_none());

        // An exited child flips the offer on: status hint plus `r`.
        let dead = EmbeddedPty::spawn("true", &[], 80, 24).unwrap();
        view.ptys.insert(id.clone(), dead);
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
            "run ended · r: restart · n: new · q: quit"
        );

        // Restart drops the dead PTY and re-queues on the same id,
        // keeping the run's title (no new entry, no archive).
        let title_before = view.app.selected_session().unwrap().title.clone();
        view.restart_run();
        assert!(!view.ptys.contains_key(&id));
        assert_eq!(
            view.app.take_pending_spawn(),
            Some(crate::embedded::SpawnKind::New)
        );
        assert_eq!(view.active_id().as_deref(), Some(id.as_str()));
        assert_eq!(view.app.selected_session().unwrap().title, title_before);
        assert!(!view.can_restart());
    }

    #[test]
    fn safe_quit_is_instant_dirty_quit_arms_then_quits() {
        // Empty shell quits instantly and never arms.
        let mut view = ShellView::new();
        assert!(!view.confirm_quit_required());
        assert!(view.request_quit());
        assert!(!view.quit_armed);
        // A live PTY forces the two-step: arm first, quit second.
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        view.ptys.insert(
            id,
            EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap(),
        );
        assert!(view.confirm_quit_required());
        assert!(!view.request_quit());
        assert!(view.quit_armed);
        assert_eq!(
            view.status_text(),
            "Live runs active — q again to quit · any other key cancels"
        );
        assert!(view.request_quit());
        // Any other key disarms back to the hint.
        let mut view2 = ShellView::new();
        view2.app.start_new_session();
        let _ = view2.app.take_pending_spawn();
        let id2 = view2.active_id().unwrap();
        view2.ptys.insert(
            id2,
            EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap(),
        );
        assert!(!view2.request_quit());
        view2.nav_action("j", false);
        assert!(!view2.quit_armed);
        assert!(!view2.request_quit());
    }

    #[test]
    fn close_kills_run_entry_and_pty_without_touching_neighbors() {
        let mut view = ShellView::new();
        for _ in 0..2 {
            view.app.start_new_session();
            let _ = view.app.take_pending_spawn();
            let id = view.active_id().unwrap();
            view.ptys.insert(
                id,
                EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap(),
            );
            view.app.focus_nav();
        }
        assert_eq!(view.app.sessions.len(), 2);
        assert_eq!(view.app.selected, 1);
        assert_eq!(view.nav_action("x", false), NavAction::Close);
        view.close_run();
        // The selected run is gone — entry and PTY — the neighbor keeps
        // both, and the flash names the closed run.
        assert_eq!(view.app.sessions.len(), 1);
        assert_eq!(view.app.sessions[0].id, "run-1");
        assert_eq!(view.ptys.len(), 1);
        assert!(view.ptys.contains_key("run-1"));
        assert!(view.status_text().contains("closed"));
        // Closing the last run empties the list without panicking.
        view.close_run();
        assert!(view.app.sessions.is_empty());
        assert!(view.ptys.is_empty());
        assert!(!view.confirm_quit_required());
    }

    #[test]
    fn unchanged_screens_reuse_cached_attention() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn(
            "printf",
            &["Waiting for your approval to proceed\\n".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id.clone(), Instant::now());
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if s.status == Status::Attention {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // The scan populated the cache; the now-static screen keeps its
        // Attention status across refreshes via the cached bit alone.
        assert_eq!(view.attention_cache.get(&id), Some(&true));
        for _ in 0..10 {
            view.refresh();
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.status, Status::Attention);
        assert_eq!(view.attention_cache.get(&id), Some(&true));
    }

    #[test]
    fn exit_transition_counts_as_dirt_without_output_bytes() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        // `true` writes nothing: the only dirt is the exit itself.
        view.ptys
            .insert(id.clone(), EmbeddedPty::spawn("true", &[], 80, 24).unwrap());
        view.last_output.insert(id.clone(), Instant::now());
        let mut saw_dirt = false;
        for _ in 0..100 {
            if view.refresh() {
                saw_dirt = true;
            }
            if view.ptys[&id].view().exited {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(view.ptys[&id].view().exited);
        assert!(saw_dirt, "the exit tick must report dirt");
    }

    #[test]
    fn issue_and_file_refs_accumulate_like_pr_links() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn(
            "printf",
            &["see https://github.com/acme/app/issues/7 at src/app.rs:9\\n".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id.clone(), Instant::now());
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if !s.related_links.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            s.related_links,
            vec![
                "https://github.com/acme/app/issues/7".to_string(),
                "src/app.rs:9".to_string()
            ]
        );
    }

    #[test]
    fn pr_links_accumulate_first_seen_order_without_cap() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn(
            "printf",
            &["see https://github.com/acme/app/pull/42\\n".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id.clone(), Instant::now());
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if !s.pr_links.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // A later screen showing a new PR keeps the old one (scrolled-off
        // links are not dropped) and shows every link, uncapped.
        {
            let s = view.app.sessions.iter_mut().find(|s| s.id == id).unwrap();
            assert_eq!(s.pr_links, vec!["https://github.com/acme/app/pull/42"]);
        }
        let pty2 = EmbeddedPty::spawn(
            "printf",
            &["now https://github.com/acme/app/pull/43 and https://github.com/acme/app/pull/42\\n"
                .to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty2);
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            if s.pr_links.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            s.pr_links,
            vec![
                "https://github.com/acme/app/pull/42".to_string(),
                "https://github.com/acme/app/pull/43".to_string()
            ]
        );
    }

    #[test]
    fn selection_pair_normalizes_and_rejects_empty() {
        let mut view = ShellView::new();
        assert!(view.selection_pair().is_none());
        view.sel_anchor = Some((1, 7));
        view.sel_active = Some((1, 3));
        assert_eq!(view.selection_pair(), Some(((1, 3), (1, 7))));
        view.sel_active = Some((1, 7));
        assert!(view.selection_pair().is_none());
        view.clear_selection();
        assert!(view.selection_pair().is_none());
    }

    #[test]
    fn selected_text_uses_emulator_between_cells() {
        let mut view = ShellView::new();
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        let id = view.active_id().unwrap();
        let pty = EmbeddedPty::spawn("printf", &["hello world\\n".to_string()], 80, 24).unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id.clone(), Instant::now());
        for _ in 0..100 {
            view.refresh();
            let s = view.app.sessions.iter().find(|s| s.id == id).unwrap();
            let _ = s;
            if let Some(v) = view.active_view() {
                if v.screen.contents().contains("hello") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        view.sel_anchor = Some((0, 0));
        view.sel_active = Some((0, 5));
        assert_eq!(view.selected_text().as_deref(), Some("hello"));
        view.clear_selection();
        assert!(view.selected_text().is_none());
    }

    #[test]
    fn typed_line_becomes_run_title_on_enter_without_pty() {
        let mut view = ShellView::new();
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

    #[test]
    fn layout_text_partition_satisfies_with_runs() {
        // Regression test for the "new session" crash: gpui validates that
        // run lengths partition the text byte-exactly and panics otherwise.
        // This calls the real constructor, so it panics here first.
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[2J\x1b[1;1Htop \x1b[31mred\x1b[0m \xc3\xa9\xe2\x9d\xaf");
        let rows = super::super::terminal::screen_rows(
            parser.screen(),
            Some((0, 0)),
            super::super::terminal::Rgb8(200, 200, 200),
        );
        let (full, runs) = super::layout_text(&rows);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len(), "runs must cover every byte");
        assert!(runs.iter().all(|r| r.len > 0), "no empty runs");
        let _ = gpui::StyledText::new(full).with_runs(runs);
        // Empty screen still partitions (single covered space per row).
        let mut empty = vt100::Parser::new(24, 80, 0);
        empty.process(b"");
        let rows = super::super::terminal::screen_rows(
            empty.screen(),
            None,
            super::super::terminal::Rgb8(0, 0, 0),
        );
        let (full, runs) = super::layout_text(&rows);
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
        let pty = EmbeddedPty::spawn(
            "printf",
            &["\\x1b[2J\\x1b[1;1Hhi \\x1b[31mred\\n\"".to_string()],
            80,
            24,
        )
        .unwrap();
        view.ptys.insert(id.clone(), pty);
        view.last_output.insert(id, Instant::now());
        for _ in 0..50 {
            view.refresh();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let live = view.active_view().expect("live pty has a view");
        let _ = view.render_live_terminal(live);
    }

    #[test]
    fn empty_pane_splits_empty_vs_failed_by_icon() {
        let mut view = ShellView::new();
        // No sessions, no error: the calm empty state.
        let (icon, msg, is_error) = view.empty_pane_copy();
        assert_eq!(icon, "○");
        assert!(msg.starts_with("No session yet"));
        assert!(!is_error);
        // A recorded spawn failure flips icon, copy, and error flag so the
        // states differ by shape, not text alone.
        view.app.start_new_session();
        let _ = view.app.take_pending_spawn();
        view.app.set_error("missing binary");
        let (icon, msg, is_error) = view.empty_pane_copy();
        assert_eq!(icon, "✕");
        assert!(msg.starts_with("spawn failed:"));
        assert!(is_error);
        // Dismissal restores the empty state.
        view.app.clear_error();
        let (icon, msg, is_error) = view.empty_pane_copy();
        assert_eq!(icon, "○");
        assert!(msg.starts_with("No session yet"));
        assert!(!is_error);
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
