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
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString,
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
    None,
}

pub struct ShellView {
    app: App,
    ptys: HashMap<String, EmbeddedPty>,
    last_output: HashMap<String, Instant>,
    pending_inputs: HashMap<String, String>,
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
}

impl ShellView {
    pub fn new() -> Self {
        Self {
            app: App::new(vec![]),
            ptys: HashMap::new(),
            last_output: HashMap::new(),
            pending_inputs: HashMap::new(),
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

    /// Start the queued `muse` for the newly created run.
    fn spawn_queued(&mut self) {
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

    /// One pump iteration for the background task.
    pub fn tick(&mut self) {
        self.refresh();
    }

    /// Pump every run, refresh each entry from its live screen (attention
    /// markers, working/idle by recency, PR links), re-sort pinned.
    /// Returns true when any run produced fresh output.
    fn refresh(&mut self) -> bool {
        self.spawn_queued();
        let mut fresh_any = false;
        for (id, pty) in self.ptys.iter_mut() {
            if pty.pump() {
                fresh_any = true;
                self.last_output.insert(id.clone(), Instant::now());
            }
        }
        let now = Instant::now();
        let ids: Vec<String> = self.ptys.keys().cloned().collect();
        for id in ids {
            let pty = &self.ptys[&id];
            let view = pty.view();
            let text = view.screen.contents();
            // One classifier for live and historic runs alike (owned by
            // `app`): attention markers win, then exit, then recency.
            let age = self.last_output.get(&id).map(|at| now.duration_since(*at));
            let status = crate::app::classify(&text, age, view.exited);
            // Accumulate PR links in first-seen order: the visible screen
            // is only a viewport (vt100 `contents()` shows the live grid,
            // not full scrollback), so replacing would drop links that
            // scrolled off. Merging keeps every PR URL ever seen per run.
            let fresh = extract_pr_links(&text);
            if let Some(s) = self.app.sessions.iter_mut().find(|s| s.id == id) {
                s.status = status;
                for link in fresh {
                    if !s.pr_links.contains(&link) {
                        s.pr_links.push(link);
                    }
                }
            }
        }
        self.app.resort_keep_selection();
        fresh_any
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
    fn nav_action(&mut self, key: &str, ctrl: bool) -> NavAction {
        let action = match (key, ctrl) {
            ("q", false) | ("escape", _) => NavAction::Quit,
            ("j", false) | ("down", _) => {
                self.app.select_next();
                self.clear_selection();
                NavAction::None
            }
            ("k", false) | ("up", _) => {
                self.app.select_prev();
                self.clear_selection();
                NavAction::None
            }
            ("n", false) => {
                self.app.start_new_session();
                self.clear_selection();
                NavAction::FocusTerm
            }
            ("i", false) | ("enter", _) => NavAction::FocusTerm,
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
            NavAction::None => {}
        }
        window.refresh();
    }

    /// Resize the active PTY to the central pane, measured in monospace cells.
    fn fit_pty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        let avail_w = (f32::from(viewport.width) - LEFT_WIDTH).max(200.0);
        let avail_h = (f32::from(viewport.height) - STATUS_HEIGHT).max(120.0);
        let (char_w, line_h) = mono_metrics(cx);
        self.char_w = char_w;
        self.line_h = line_h;
        let cols = ((avail_w / char_w) as u16).clamp(20, 400);
        let rows = ((avail_h / line_h) as u16).clamp(10, 200);
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
                    // PR links as child items: click copies the full URL.
                    let links: Vec<SidebarMenuItem> = s
                        .pr_links
                        .iter()
                        .map(|link| {
                            let short = link
                                .trim_start_matches("https://")
                                .trim_start_matches("http://")
                                .trim_start_matches("github.com/")
                                .to_string();
                            let url = link.clone();
                            SidebarMenuItem::new(short).on_click(cx.listener(
                                move |this, _ev, _window, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                    this.app.set_status("copied PR link");
                                },
                            ))
                        })
                        .collect();
                    SidebarMenuItem::new(format!("{row_marker} {}", s.title))
                        .active(i == self.app.selected)
                        .default_open(true)
                        .on_click(cx.listener(move |this, _ev, window, _cx| {
                            if let Some(pos) = this.app.sessions.iter().position(|s| s.id == row_id)
                            {
                                this.app.selected = pos;
                            }
                            this.clear_selection();
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

    fn render_terminal(&self) -> gpui::Div {
        let Some(view) = self.active_view() else {
            return div()
                .flex_1()
                .h_full()
                .p_4()
                .child(
                    div()
                        .text_color(rgb(0x888888))
                        .child(match self.app.error_text() {
                            Some(e) => format!("spawn failed: {e}"),
                            None => "No session yet. Press n to start a new muse.".to_string(),
                        }),
                );
        };
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
        if self.app.is_terminal_focused() {
            "typing in muse · Tab/Esc: sessions · drag: select · Cmd+C: copy · Cmd/Ctrl+V: paste"
                .to_string()
        } else if self.app.sessions.is_empty() {
            "n: new muse · q: quit".to_string()
        } else {
            "n: new · j/k: move · Tab/i: type · x: close · drag: select · y: copy · p: paste · q: quit"
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
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .child(self.render_runs(cx))
                    .child(
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
                            .child(self.render_terminal())
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
                            .on_mouse_move(cx.listener(
                                |this, ev: &MouseMoveEvent, _window, _cx| {
                                    this.update_selection(ev.position);
                                },
                            ))
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
                    .child(self.status_text())
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

/// Window options for the main window.
pub fn window_options() -> WindowOptions {
    WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(SharedString::from("Agent Manager")),
            ..Default::default()
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
        let _ = view.render_terminal();
    }

    #[test]
    fn gpui_version_is_pinned() {
        // Fails loudly on upgrade: review gpui API changes consciously.
        let lock = std::fs::read_to_string("Cargo.lock").expect("Cargo.lock readable in tests");
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
    fn chrome_stays_on_the_gpui_02_component_line() {
        // The sessions chrome uses gpui-component 0.5.x, the last line built
        // on gpui 0.2.2. 0.6+ moved to the gpui-pre 0.3.6 fork and would
        // force a framework migration: fail loudly so that move is conscious.
        let lock = std::fs::read_to_string("Cargo.lock").expect("Cargo.lock readable in tests");
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
