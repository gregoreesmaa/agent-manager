//! Window composition for [`super::shell::ShellView`].
//!
//! The root [`Render`] impl: runs panel beside the terminal column
//! (header, error banner, terminal), plus the status bar and window
//! options. Event wiring lives in [`super::shell`]; panel and terminal
//! rendering live in [`super::runs_panel`] and [`super::terminal_pane`].

use gpui::{
    div, px, rgb, Context, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, WindowOptions,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::Sizable as _;

use super::shell::{ShellView, STATUS_HEIGHT};

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
