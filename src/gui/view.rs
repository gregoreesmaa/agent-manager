//! Window composition for [`super::shell::ShellView`].
//!
//! The root [`Render`] impl: runs panel beside the terminal column
//! (header, error banner, terminal), plus the status bar, window options,
//! and viewport-driven pane geometry (`fit_pty`). Event wiring lives in
//! [`super::shell`]; panel and terminal rendering live in
//! [`super::runs_panel`] and [`super::terminal_pane`]; pure geometry lives
//! in [`super::layout`].

use gpui::{
    div, font, px, rgb, Context, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, SharedString, Size,
    StatefulInteractiveElement, Styled, Window, WindowOptions,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::Sizable as _;

use super::layout::{
    effective_sidebar_width, sidebar_visible_for_width, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    STATUS_HEIGHT,
};
use super::shell::ShellView;

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
        let term_title_color = if typing {
            rgb(0xffd866)
        } else {
            rgb(super::theme::SECONDARY_FG)
        };

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
                        // In-app help (`?` toggle) replaces the terminal
                        // pane while open; `?` again (or Close) returns.
                        .child(if self.show_help {
                            self.render_help(cx)
                        } else {
                            self.render_terminal(cx)
                        })
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
                    .text_color(rgb(super::theme::SECONDARY_FG))
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

impl ShellView {
    /// Resize the active PTY to the central pane, measured in monospace cells.
    /// Narrow viewports (<700px) collapse the sidebar, so the pane (and the
    /// PTY) use the full window width instead of squeezing beside 264px.
    pub(crate) fn fit_pty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        let avail_w =
            f32::from(viewport.width) - effective_sidebar_width(f32::from(viewport.width));
        let avail_h = f32::from(viewport.height) - STATUS_HEIGHT;
        // Metrics come from the font cache: only the grid math below
        // re-runs per frame, never the font-system measure.
        let (char_w, line_h) = self.cached_mono_metrics(cx);
        self.char_w = char_w;
        self.line_h = line_h;
        let (cols, rows) = super::layout::pty_grid_for(avail_w, avail_h, char_w, line_h);
        if (cols, rows) != (self.cols, self.rows) {
            self.cols = cols;
            self.rows = rows;
            if let Some(id) = self.active_id() {
                if let Some(run) = self.runs.get_mut(&id) {
                    run.pty.resize(cols, rows);
                }
            }
        }
    }
}

impl ShellView {
    /// Font-system metrics, cached per configured font (issue #35): the
    /// text system is app-global rather than per-window, so resizes and
    /// display moves change the grid math in [`Self::fit_pty`] but never
    /// re-measure — unless the configured family/size changed, which
    /// drops the stale entry.
    fn cached_mono_metrics(&mut self, cx: &mut Context<ShellView>) -> (f32, f32) {
        let family = self.app.terminal_config().font_family.clone();
        let size = self.term_font_size();
        let key = (family.clone(), size.to_bits());
        if let Some((k, m)) = self.mono_metrics.as_ref() {
            if *k == key {
                return *m;
            }
        }
        let metrics = mono_metrics(cx, &family, size);
        self.mono_metrics = Some((key, metrics));
        metrics
    }
}

/// Monospace cell metrics for PTY sizing, with sane fallbacks.
/// `family`/`size` come from the terminal config (issue #35) via
/// [`ShellView::cached_mono_metrics`] so the PTY grid matches what the
/// pane actually renders.
pub(crate) fn mono_metrics(cx: &mut Context<ShellView>, family: &str, size: f32) -> (f32, f32) {
    let size = px(size);
    let system = cx.text_system();
    let fid = system.resolve_font(&font(family.to_string()));
    let char_w = system
        .advance(fid, size, ' ')
        .map(|s| f32::from(s.width))
        .unwrap_or(8.0);
    let line_h = f32::from(system.ascent(fid, size)) + f32::from(system.descent(fid, size));
    let line_h = if line_h > 0.0 { line_h } else { 18.0 };
    (char_w.max(4.0), line_h.max(8.0))
}

impl ShellView {
    /// In-app help panel: the full keymap as text rows plus a Close button,
    /// dismissed by `?` — the keymap no longer lives only in the README.
    pub(crate) fn render_help(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut col = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(rgb(0x1e1e2e))
            .text_color(super::terminal::to_hsla(super::shell::DEFAULT_FG))
            .text_sm()
            .child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .child("Keys — press ? to close".to_string()),
            );
        for (key, what) in super::nav::help_entries() {
            col = col.child(format!("{key}   {what}"));
        }
        col.child(
            Button::new(ElementId::Name("help-close-btn".into()))
                .label("Close (?)")
                .small()
                .on_click(cx.listener(|this, _ev, _window, _cx| {
                    this.show_help = false;
                })),
        )
    }

    /// First-run orientation copy: icon + message + error flag so the
    /// empty state (`No session yet`) and the failure state (`spawn
    /// failed`) differ by shape and color, not text alone.
    pub(crate) fn empty_pane_copy(&self) -> (&'static str, String, bool) {
        match self.app.error_text() {
            Some(e) => ("✕", format!("spawn failed: {e}"), true),
            None => (
                "○",
                // The effective spawn command is the UI affordance for
                // issue #33: configured flags are visible before launch.
                format!(
                    "No session yet. Press n to start: {}. Press ? for keys.",
                    self.app.spawn_command_string()
                ),
                false,
            ),
        }
    }

    /// Empty terminal pane: icon-split status line plus a clickable
    /// `+ New` CTA (keyboard parity: `n` still works) so first run does
    /// not depend on discovering the key hint.
    pub(crate) fn render_empty_pane(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (icon, msg, is_error) = self.empty_pane_copy();
        let color = if is_error {
            rgb(0xff9999)
        } else {
            rgb(super::theme::SECONDARY_FG)
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
    use super::super::layout::{
        FALLBACK_CHAR_W, FALLBACK_LINE_H, HEADER_HEIGHT, LEFT_WIDTH, MIN_COLS, MIN_ROWS,
        STATUS_HEIGHT,
    };
    use super::super::runs::test_shell;
    use super::*;

    #[test]
    fn window_min_size_matches_pty_floors() {
        // Issue #6: the OS minimum must fit the PTY floors, not clip them.
        let min = super::window_options()
            .window_min_size
            .expect("main window sets a minimum size");
        assert_eq!(f32::from(min.width), MIN_WINDOW_WIDTH);
        assert_eq!(f32::from(min.height), MIN_WINDOW_HEIGHT);
        // At the minimum size the terminal pane still fits a full
        // MIN_COLS x MIN_ROWS grid at fallback metrics.
        let (cols, rows) = super::super::layout::pty_grid_for(
            f32::from(min.width) - LEFT_WIDTH,
            f32::from(min.height) - STATUS_HEIGHT - HEADER_HEIGHT,
            FALLBACK_CHAR_W,
            FALLBACK_LINE_H,
        );
        assert!(cols >= MIN_COLS, "min width fits {cols} cols");
        assert!(rows >= MIN_ROWS, "min height fits {rows} rows");
    }

    #[test]
    fn empty_pane_advertises_configured_spawn_flags() {
        // Issue #33: the empty pane names the effective spawn command so
        // configured flags are visible before launch.
        use crate::config::{AgentConfig, Config};
        let view = test_shell();
        let (_, msg, _) = view.empty_pane_copy();
        assert!(msg.starts_with("No session yet"));
        assert!(msg.contains("start: muse."), "plain spawn: {msg:?}");
        let mut flagged = test_shell();
        let mut cfg = Config::default();
        cfg.agents.insert(
            "muse".to_string(),
            AgentConfig {
                extra_args: vec!["--yolo".to_string()],
            },
        );
        flagged.app.set_config(cfg);
        let (_, msg, _) = flagged.empty_pane_copy();
        assert!(msg.contains("muse --yolo"), "flags shown: {msg:?}");
    }

    #[test]
    fn empty_pane_splits_empty_vs_failed_by_icon() {
        let mut view = test_shell();
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
}
