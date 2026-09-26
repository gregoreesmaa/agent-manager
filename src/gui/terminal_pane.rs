//! Terminal-pane render + mouse selection for [`super::shell::ShellView`].
//!
//! Hand-rolled gpui terminal: vt100 screen → styled rows, drag-to-select
//! with copy-on-select, clipboard copy/paste, the sticky-error banner,
//! the first-run empty pane, PTY sizing, and the text-partition helper
//! behind it. Framework-free mapping details stay in [`super::terminal`].

use gpui::{
    div, px, rgb, App as GpuiApp, ClipboardItem, Context, ElementId, Font, FontFallbacks,
    ParentElement, Pixels, Point, Styled, StyledText, TextRun, UnderlineStyle,
};
use gpui_component::button::{Button, ButtonVariants as _};

use crate::config::TerminalConfig;
use crate::embedded::LiveView;

use super::shell::{ShellView, CURSOR_BG, DEFAULT_FG, SELECTION_BG};
use super::terminal::{
    point_to_cell, screen_fingerprint, screen_rows, selection_rows, selection_text, to_hsla,
    CellPos,
};
use gpui_component::Sizable as _;

/// Cursor cell the frame renders, if any: exited runs and hidden cursors
/// paint no caret. Shared by the render path and the cache fingerprint
/// so the key can never disagree with the pixels.
pub(crate) fn resolved_cursor(view: &LiveView) -> Option<CellPos> {
    if view.exited || view.screen.hide_cursor() {
        None
    } else {
        Some(view.screen.cursor_position())
    }
}

/// One cached terminal frame: the flattened text + gpui runs for a
/// (run id, screen fingerprint) key. Single-entry: a miss overwrites, so
/// memory stays flat and run switches invalidate by construction.
#[derive(Default)]
pub(crate) struct TermFrameCache {
    key: Option<(String, u64)>,
    full: String,
    runs: Vec<TextRun>,
}

impl TermFrameCache {
    /// Hit: hand back a clone of the cached frame. `StyledText` takes
    /// ownership per repaint, so one `String` + one `Vec` clone is the
    /// per-frame price — not a full grid rebuild.
    pub(crate) fn get(&self, run_id: &str, fingerprint: u64) -> Option<(String, Vec<TextRun>)> {
        if self
            .key
            .as_ref()
            .is_some_and(|(id, f)| id == run_id && *f == fingerprint)
        {
            Some((self.full.clone(), self.runs.clone()))
        } else {
            None
        }
    }

    /// Miss: remember this frame, dropping whatever was cached.
    pub(crate) fn store(
        &mut self,
        run_id: String,
        fingerprint: u64,
        full: String,
        runs: Vec<TextRun>,
    ) {
        self.key = Some((run_id, fingerprint));
        self.full = full;
        self.runs = runs;
    }
}

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
        // A paged run copies from the visible pager slice (issue #25).
        if let Some(text) = self.selected_pager_text().or_else(|| self.selected_text()) {
            let chars = text.chars().count();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.app
                .set_status(format!("copied selection ({chars} chars)"));
        } else {
            self.clear_selection();
        }
    }

    pub(crate) fn render_terminal(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        // Pager (issue #25): a scrolled-up run shows its retained slice
        // instead of the live grid. The offset moves with no screen
        // change, so this bypasses the fingerprint cache below.
        if let Some(spans) = self.pager_spans() {
            let cols = self
                .active_view()
                .map(|v| v.screen.size().1)
                .unwrap_or(self.cols);
            let term_font = terminal_font(self.app.terminal_config());
            let (full, runs) = layout_text(&spans, &term_font);
            return self.assemble_live_terminal(full, runs, cols);
        }
        // Cache the flattened frame per (run, screen fingerprint): an
        // unchanged screen skips the `screen_rows` + `layout_text` rebuild
        // (thousands of allocations) on every repaint and reuses the
        // cloned text + runs instead.
        let (run_id, fingerprint, cols) = match (self.active_id(), self.active_view()) {
            (Some(id), Some(view)) => {
                let fingerprint = screen_fingerprint(view.screen, resolved_cursor(&view));
                let (_, cols) = view.screen.size();
                (id, fingerprint, cols)
            }
            _ => return self.render_empty_pane(cx),
        };
        if let Some((full, runs)) = self.term_frame.get(&run_id, fingerprint) {
            return self.assemble_live_terminal(full, runs, cols);
        }
        let Some(view) = self.active_view() else {
            return self.render_empty_pane(cx);
        };
        let rows = screen_rows(view.screen, resolved_cursor(&view), CURSOR_BG);
        let term_font = terminal_font(self.app.terminal_config());
        let (full, runs) = layout_text(&rows, &term_font);
        self.term_frame
            .store(run_id, fingerprint, full.clone(), runs.clone());
        self.assemble_live_terminal(full, runs, cols)
    }

    /// Assemble the terminal element from flattened text + runs: the
    /// `StyledText` plus the mouse-selection highlight behind it. Takes
    /// no window context so headless tests can build the element without
    /// a gpui harness; production always reaches it through the cached
    /// [`Self::render_terminal`] path.
    fn assemble_live_terminal(&self, full: String, runs: Vec<TextRun>, cols: u16) -> gpui::Div {
        let text = StyledText::new(full).with_runs(runs);
        // Mouse selection highlight: cell rectangles behind the text. The
        // inner wrapper has no padding, so overlay origin == text origin and
        // no padding constant is needed.
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
        let stack = self.app.terminal_config().font_stack();
        div()
            .flex_1()
            .h_full()
            .bg(rgb(0x11111b))
            .p_2()
            .font_family(stack[0].clone())
            .text_size(px(self.term_font_size()))
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

/// Build the gpui [`Font`] for the terminal pane from the user setting
/// (issue #35): the configured primary family plus the explicit
/// emoji/CJK/monospace fallback chain, so one missing family never
/// silently changes metrics mid-row.
pub(crate) fn terminal_font(cfg: &TerminalConfig) -> Font {
    let stack = cfg.font_stack();
    let mut fallbacks = stack.clone();
    fallbacks.remove(0);
    Font {
        family: stack[0].clone().into(),
        features: Default::default(),
        weight: Default::default(),
        style: Default::default(),
        fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
    }
}

/// Flatten screen rows into one string plus gpui text runs. Every byte of
/// the string belongs to exactly one non-empty run — gpui validates this
/// partition and panics otherwise (crashed the first launch).
pub(crate) fn layout_text(
    rows: &[Vec<super::terminal::TermSpan>],
    mono: &Font,
) -> (String, Vec<TextRun>) {
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
        use crate::config::TerminalConfig;
        let mono = terminal_font(&TerminalConfig::default());
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[2J\x1b[1;1Htop \x1b[31mred\x1b[0m \xc3\xa9\xe2\x9d\xaf");
        let rows = screen_rows(parser.screen(), Some((0, 0)), Rgb8(200, 200, 200));
        let (full, runs) = layout_text(&rows, &mono);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len(), "runs must cover every byte");
        assert!(runs.iter().all(|r| r.len > 0), "no empty runs");
        let _ = gpui::StyledText::new(full).with_runs(runs);
        // Empty screen still partitions (single covered space per row).
        let mut empty = vt100::Parser::new(24, 80, 0);
        empty.process(b"");
        let rows = screen_rows(empty.screen(), None, Rgb8(0, 0, 0));
        let (full, runs) = layout_text(&rows, &mono);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len());
        let _ = gpui::StyledText::new(full).with_runs(runs);
    }

    #[test]
    fn terminal_font_heads_primary_with_fallback_chain() {
        // Issue #35: the gpui font carries the configured primary plus
        // the explicit fallback list; an override swaps only the head.
        use crate::config::TerminalConfig;
        let head = terminal_font(&TerminalConfig::default());
        assert_eq!(head.family.as_ref(), "JetBrainsMono Nerd Font");
        let fallbacks = head
            .fallbacks
            .expect("fallback chain is explicit")
            .fallback_list()
            .to_vec();
        assert!(fallbacks.contains(&"Apple Color Emoji".to_string()));
        assert!(fallbacks.contains(&"Noto Sans Mono CJK SC".to_string()));
        assert!(!fallbacks.contains(&"JetBrainsMono Nerd Font".to_string()));
        let custom = TerminalConfig {
            font_family: "Iosevka Nerd Font".to_string(),
            ..TerminalConfig::default()
        };
        let swapped = terminal_font(&custom);
        assert_eq!(swapped.family.as_ref(), "Iosevka Nerd Font");
        let fallbacks = swapped
            .fallbacks
            .as_ref()
            .expect("fallbacks survive override")
            .fallback_list()
            .to_vec();
        assert!(fallbacks.contains(&"Apple Color Emoji".to_string()));
        // Bold keeps the same chain (styled spans must not change metrics).
        assert_eq!(swapped.clone().bold().family, swapped.family);
        assert_eq!(swapped.clone().bold().fallbacks, swapped.fallbacks);
    }

    #[test]
    fn coverage_fixture_renders_without_tofu_or_column_drift() {
        // Issue #35 acceptance fixture: every glyph class the CLIs can
        // emit passes through screen_rows → layout_text intact, partitions
        // byte-exactly, and wide chars still occupy two cells.
        use crate::config::TerminalConfig;
        let mono = terminal_font(&TerminalConfig::default());
        let mut parser = vt100::Parser::new(24, 100, 0);
        // Box-drawing, blocks/shades, powerline + Nerd Font icons,
        // bold/italic styles, CJK, and emoji.
        let fixture = "─│┌┐└┘├┤┬┴┼ █▉▊▋▌▍▎▏▓▒░▀▄ \u{e0b0}\u{e0b1}\u{e0b2}  \u{f0244} \x1b[1mbold\x1b[0m \x1b[3mital\x1b[0m 你好世界 \u{1f600}";
        parser.process(fixture.as_bytes());
        let screen = parser.screen();
        let contents = screen.contents();
        for needle in ["─│┌┐", "▓▒░▀", "bold", "ital", "你好世界"] {
            assert!(
                contents.contains(needle),
                "fixture keeps {needle:?}: {contents:?}"
            );
        }
        // Wide chars (CJK/emoji) occupy two cells: a continuation cell
        // follows each lead, so the vt100 column grid cannot drift.
        let mut saw_continuation = false;
        let (rows, cols) = screen.size();
        for r in 0..rows {
            for c in 0..cols {
                if let Some(cell) = screen.cell(r, c) {
                    if cell.is_wide_continuation() {
                        saw_continuation = true;
                    }
                }
            }
        }
        assert!(saw_continuation, "wide chars take two cells");
        // And the whole grid still partitions for gpui.
        let grid = screen_rows(screen, None, Rgb8(200, 200, 200));
        let (full, runs) = layout_text(&grid, &mono);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, full.len());
        let _ = gpui::StyledText::new(full).with_runs(runs);
    }

    #[test]
    fn frame_cache_hits_reuse_output_and_misses_on_change_or_run() {
        use super::super::terminal::screen_fingerprint;
        let mut cache = TermFrameCache::default();
        assert!(cache.get("run-1", 42).is_none());
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"hello");
        let fp = screen_fingerprint(parser.screen(), Some((0, 5)));
        let rows = screen_rows(parser.screen(), Some((0, 5)), Rgb8(0, 0, 0));
        let mono = terminal_font(&TerminalConfig::default());
        let (full, runs) = layout_text(&rows, &mono);
        cache.store("run-1".to_string(), fp, full.clone(), runs.clone());
        // Same run + fingerprint: the cached text and runs come back
        // byte-identical (the repaint reuses them instead of rebuilding).
        let (hit_full, hit_runs) = cache.get("run-1", fp).expect("cache hit");
        assert_eq!(hit_full, full);
        assert_eq!(hit_runs, runs);
        // A different run never aliases, even with an identical screen.
        assert!(cache.get("run-2", fp).is_none());
        // New output misses under the old key: the caller rebuilds and
        // the store drops the stale frame.
        parser.process(b"!");
        let fp2 = screen_fingerprint(parser.screen(), Some((0, 6)));
        assert_ne!(fp2, fp);
        assert!(cache.get("run-1", fp2).is_none());
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
        // Same pieces the cached path assembles: rows, layout, element.
        let rows = screen_rows(live.screen, resolved_cursor(&live), CURSOR_BG);
        let mono = terminal_font(&TerminalConfig::default());
        let (full, runs) = layout_text(&rows, &mono);
        let (_, cols) = live.screen.size();
        let _ = view.assemble_live_terminal(full, runs, cols);
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
