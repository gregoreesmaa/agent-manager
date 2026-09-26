//! Terminal-pane render + mouse selection for [`super::shell::ShellView`].
//!
//! Hand-rolled gpui terminal: vt100 screen → styled rows, drag-to-select
//! with copy-on-select, clipboard copy/paste, the sticky-error banner,
//! the first-run empty pane, PTY sizing, and the text-partition helper
//! behind it. Framework-free mapping details stay in [`super::terminal`].

use gpui::{
    div, font, px, rgb, App as GpuiApp, ClipboardItem, Context, ElementId, ParentElement, Pixels,
    Point, Styled, StyledText, TextRun, UnderlineStyle,
};
use gpui_component::button::{Button, ButtonVariants as _};

use crate::embedded::LiveView;

use super::shell::{ShellView, CURSOR_BG, DEFAULT_FG, SELECTION_BG, TERM_FONT_SIZE};
use super::terminal::{
    point_to_cell, screen_rows, selection_rows, selection_text, to_hsla, CellPos,
};
use gpui_component::Sizable as _;

impl ShellView {
    /// Normalized, non-empty mouse selection in terminal cells, if any.
    pub(crate) fn selection_pair(&self) -> Option<(CellPos, CellPos)> {
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
    pub(crate) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_pair()?;
        let view = self.active_view()?;
        let text = selection_text(view.screen, start, end);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.sel_active = None;
        self.selecting = false;
    }

    /// Map a window-space mouse point to a terminal cell using the captured
    /// text bounds plus the measured cell size and live grid size.
    pub(crate) fn mouse_cell(&self, pos: Point<Pixels>) -> Option<CellPos> {
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

    pub(crate) fn begin_selection(&mut self, pos: Point<Pixels>) {
        if let Some(cell) = self.mouse_cell(pos) {
            self.sel_anchor = Some(cell);
            self.sel_active = Some(cell);
            self.selecting = true;
        } else {
            self.clear_selection();
        }
    }

    pub(crate) fn update_selection(&mut self, pos: Point<Pixels>) {
        if !self.selecting {
            return;
        }
        if let Some(cell) = self.mouse_cell(pos) {
            self.sel_active = Some(cell);
        }
    }

    /// Finish a drag: copy-on-select when the drag covered text, mirroring
    /// terminal copy-on-select behavior.
    pub(crate) fn end_selection(&mut self, pos: Point<Pixels>, cx: &mut GpuiApp) {
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

    pub(crate) fn render_terminal(&self, cx: &mut Context<Self>) -> gpui::Div {
        super::theme::debug_assert_contrast();
        let Some(view) = self.active_view() else {
            return self.render_empty_pane(cx);
        };
        self.render_live_terminal(view)
    }

    /// Live terminal pane. Takes no window context so headless tests can
    /// build the element without a gpui harness.
    pub(crate) fn render_live_terminal(&self, view: LiveView<'_>) -> gpui::Div {
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
        let slot = std::rc::Rc::clone(&self.term_text_bounds);
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
            .bg(rgb(super::theme::SURFACE_BG))
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
    pub(crate) fn render_error_banner(&self, cx: &mut Context<Self>) -> gpui::Div {
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
}

/// Flatten screen rows into one string plus gpui text runs. Every byte of
/// the string belongs to exactly one non-empty run — gpui validates this
/// partition and panics otherwise (crashed the first launch).
pub(crate) fn layout_text(rows: &[Vec<super::terminal::TermSpan>]) -> (String, Vec<TextRun>) {
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

#[cfg(test)]
mod tests {
    use super::super::runs::{insert_test_pty, test_shell};
    use super::super::shell::ShellView;
    use super::super::terminal::{screen_rows, Rgb8};
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
        view.runs
            .insert(id.clone(), super::super::runs::Run::new(pty));
        view.runs.get_mut(&id).unwrap().last_output = std::time::Instant::now();
        for _ in 0..50 {
            view.refresh();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let live = view.active_view().expect("live pty has a view");
        let _ = view.render_live_terminal(live);
    }

    #[test]
    fn selection_pair_normalizes_and_rejects_empty() {
        let mut view = test_shell();
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
        let mut view = test_shell();
        let id = insert_test_pty(&mut view, "printf", &["hello world\\n"]);
        for _ in 0..100 {
            view.refresh();
            if let Some(v) = view.active_view() {
                if v.screen.contents().contains("hello") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = id;
        view.sel_anchor = Some((0, 0));
        view.sel_active = Some((0, 5));
        assert_eq!(view.selected_text().as_deref(), Some("hello"));
        view.clear_selection();
        assert!(view.selected_text().is_none());
    }
}
