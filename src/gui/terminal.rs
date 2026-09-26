//! vt100 screen → styled text rows for the gpui terminal pane.
//!
//! Framework-light by design (regression shield): everything here works on
//! plain vt100 types plus one tiny gpui conversion ([`to_hsla`]); every
//! mapping below has a unit test pinning it, so a gpui upgrade cannot
//! silently change terminal rendering.

/// 8-bit RGB triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb8(pub u8, pub u8, pub u8);

/// Cell style in framework-free form (`None` = terminal default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: Option<Rgb8>,
    pub bg: Option<Rgb8>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// One coalesced same-style span of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermSpan {
    pub text: String,
    pub style: CellStyle,
}

/// Standard xterm 16-color palette as RGB triples.
const PALETTE_16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Map a vt100 color to RGB (`None` = terminal default).
pub fn vt_color(color: vt100::Color) -> Option<Rgb8> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => {
            let i = i as usize;
            if i < 16 {
                let (r, g, b) = PALETTE_16[i];
                Some(Rgb8(r, g, b))
            } else if i < 232 {
                let v = i - 16;
                let comp = |c: usize| {
                    if c == 0 {
                        0
                    } else {
                        (55 + 40 * c) as u8
                    }
                };
                Some(Rgb8(comp(v / 36), comp((v % 36) / 6), comp(v % 6)))
            } else {
                let g = (8 + 10 * (i - 232)) as u8;
                Some(Rgb8(g, g, g))
            }
        }
        vt100::Color::Rgb(r, g, b) => Some(Rgb8(r, g, b)),
    }
}

/// Convert RGB to gpui HSL. Kept in-house (and tested) so terminal colors do
/// not depend on any gpui helper surviving upgrades.
pub fn to_hsla(Rgb8(r, g, b): Rgb8) -> gpui::Hsla {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return gpui::Hsla {
            h: 0.0,
            s: 0.0,
            l,
            a: 1.0,
        };
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    gpui::Hsla { h, s, l, a: 1.0 }
}

/// Terminal cell position: 0-based `(row, col)`. `col` may equal the grid
/// width to denote end-of-line (exclusive selection edge).
pub type CellPos = (u16, u16);

/// Order two cell endpoints so the first is the selection start (top, then
/// left). Pure so mouse-drag direction never matters to callers.
pub fn normalize_selection(a: CellPos, b: CellPos) -> (CellPos, CellPos) {
    if (a.0, a.1) <= (b.0, b.1) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Map a window-space point to a terminal cell. `origin` is the window-space
/// top-left of the terminal text (the StyledText bounds), `char_w`/`line_h`
/// the monospace cell size, `cols`/`rows` the live grid size. The result is
/// clamped into the grid; `col` may be `cols` (exclusive line end) so a drag
/// past the right edge still selects to end-of-line.
pub fn point_to_cell(
    mouse: (f32, f32),
    origin: (f32, f32),
    char_w: f32,
    line_h: f32,
    cols: u16,
    rows: u16,
) -> CellPos {
    if char_w <= 0.0 || line_h <= 0.0 || cols == 0 || rows == 0 {
        return (0, 0);
    }
    let row = ((mouse.1 - origin.1) / line_h).floor() as i32;
    let col = ((mouse.0 - origin.0) / char_w).floor() as i32;
    let row = row.clamp(0, rows as i32 - 1) as u16;
    let col = col.clamp(0, cols as i32) as u16;
    (row, col)
}

/// Split a normalized multi-row selection into per-row `(row, start_col,
/// end_col)` spans with exclusive ends, for highlight overlays and tests.
/// `cols` is the grid width used to fill interior rows.
pub fn selection_rows(start: CellPos, end: CellPos, cols: u16) -> Vec<(u16, u16, u16)> {
    let (start, end) = normalize_selection(start, end);
    if start == end {
        return vec![];
    }
    if start.0 == end.0 {
        return vec![(start.0, start.1.min(cols), end.1.min(cols))];
    }
    let mut out = Vec::new();
    out.push((start.0, start.1.min(cols), cols));
    for r in start.0.saturating_add(1)..end.0 {
        out.push((r, 0, cols));
    }
    out.push((end.0, 0, end.1.min(cols)));
    out
}

/// Selected terminal text between two cells, via the emulator's own
/// [`vt100::Screen::contents_between`] (handles wrapping and wide cells).
/// Returns an empty string for an empty (same-cell) selection.
pub fn selection_text(screen: &vt100::Screen, anchor: CellPos, active: CellPos) -> String {
    let (start, end) = normalize_selection(anchor, active);
    if start == end {
        return String::new();
    }
    screen.contents_between(start.0, start.1, end.0, end.1)
}

/// Fingerprint of everything [`screen_rows`] renders: grid size, the
/// resolved cursor, and the formatted grid (text plus styles, in one
/// allocation). vt100 exposes no generation counter, so the render cache
/// keys on this instead of rebuilding rows (thousands of allocations)
/// every frame. Hashing the *formatted* grid — not plain `contents()` —
/// keeps style-only changes (a moved highlight with identical text) from
/// going stale.
pub fn screen_fingerprint(screen: &vt100::Screen, cursor: Option<(u16, u16)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    screen.size().hash(&mut h);
    cursor.hash(&mut h);
    screen.contents_formatted().hash(&mut h);
    h.finish()
}

/// Render the emulated screen grid as rows of coalesced spans. `cursor` is
/// the 0-based cursor cell, painted with `cursor_bg` when given (this is how
/// the child tool's caret stays visible).
pub fn screen_rows(
    screen: &vt100::Screen,
    cursor: Option<(u16, u16)>,
    cursor_bg: Rgb8,
) -> Vec<Vec<TermSpan>> {
    let (rows, cols) = screen.size();
    let mut out = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut spans: Vec<TermSpan> = Vec::new();
        let mut buf = String::new();
        let mut cur = CellStyle {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
        };
        let mut open = false;
        for c in 0..cols {
            let Some(cell) = screen.cell(r, c) else {
                break;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let (mut fg, mut bg) = (vt_color(cell.fgcolor()), vt_color(cell.bgcolor()));
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cursor == Some((r, c)) {
                bg = Some(cursor_bg);
            }
            let style = CellStyle {
                fg,
                bg,
                bold: cell.bold(),
                italic: cell.italic(),
                underline: cell.underline(),
            };
            if !open {
                cur = style;
                open = true;
            } else if style != cur {
                spans.push(TermSpan {
                    text: std::mem::take(&mut buf),
                    style: cur,
                });
                cur = style;
            }
            buf.push_str(&cell.contents());
        }
        if open {
            spans.push(TermSpan {
                text: buf,
                style: cur,
            });
        }
        out.push(spans);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_red_maps_to_xterm_value() {
        assert_eq!(vt_color(vt100::Color::Idx(1)), Some(Rgb8(205, 0, 0)));
        assert_eq!(vt_color(vt100::Color::Idx(9)), Some(Rgb8(255, 0, 0)));
    }

    #[test]
    fn default_maps_to_none_and_rgb_passes_through() {
        assert_eq!(vt_color(vt100::Color::Default), None);
        assert_eq!(vt_color(vt100::Color::Rgb(1, 2, 3)), Some(Rgb8(1, 2, 3)));
    }

    #[test]
    fn cube_and_grayscale_ramps() {
        // 196 = 5,0,0 → pure red; 232 = first gray step.
        assert_eq!(vt_color(vt100::Color::Idx(196)), Some(Rgb8(255, 0, 0)));
        assert_eq!(vt_color(vt100::Color::Idx(232)), Some(Rgb8(8, 8, 8)));
        assert_eq!(vt_color(vt100::Color::Idx(255)), Some(Rgb8(238, 238, 238)));
    }

    #[test]
    fn rgb_to_hsl_primaries() {
        let gpui::Hsla { h, s, l, .. } = to_hsla(Rgb8(255, 0, 0));
        assert!((h - 0.0).abs() < 1e-5 && (s - 1.0).abs() < 1e-5 && (l - 0.5).abs() < 1e-5);
        let gpui::Hsla { s, l, .. } = to_hsla(Rgb8(255, 255, 255));
        assert!((s - 0.0).abs() < 1e-5 && (l - 1.0).abs() < 1e-5);
        let gpui::Hsla { l, .. } = to_hsla(Rgb8(0, 0, 0));
        assert!((l - 0.0).abs() < 1e-5);
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive_to_text_style_and_cursor() {
        let mut a = vt100::Parser::new(24, 80, 0);
        a.process(b"hello");
        let mut b = vt100::Parser::new(24, 80, 0);
        b.process(b"hello");
        // Same bytes, same cursor: identical fingerprint (cache hit).
        assert_eq!(
            screen_fingerprint(a.screen(), Some((0, 5))),
            screen_fingerprint(b.screen(), Some((0, 5)))
        );
        // New text changes it.
        b.process(b"!");
        assert_ne!(
            screen_fingerprint(a.screen(), Some((0, 5))),
            screen_fingerprint(b.screen(), Some((0, 6)))
        );
        // A moved cursor alone changes it (the caret cell repaints).
        let mut c = vt100::Parser::new(24, 80, 0);
        c.process(b"hello");
        assert_ne!(
            screen_fingerprint(a.screen(), Some((0, 5))),
            screen_fingerprint(c.screen(), Some((0, 0)))
        );
        // Style-only change with identical text changes it: recoloring
        // "hello" red must not reuse the unstyled frame.
        let mut d = vt100::Parser::new(24, 80, 0);
        d.process(b"\x1b[31mhello\x1b[0m");
        assert_ne!(
            screen_fingerprint(a.screen(), None),
            screen_fingerprint(d.screen(), None)
        );
    }

    #[test]
    fn addressed_text_lands_in_its_row() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[2J\x1b[1;1Htop-left\x1b[10;20Hmid");
        let rows = screen_rows(parser.screen(), None, Rgb8(255, 255, 255));
        let row0: String = rows[0].iter().map(|s| s.text.as_str()).collect();
        let row9: String = rows[9].iter().map(|s| s.text.as_str()).collect();
        assert!(row0.contains("top-left"), "row0 was {row0:?}");
        assert!(row9.contains("mid"), "row9 was {row9:?}");
    }

    #[test]
    fn inverse_swaps_fg_to_bg() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[31;7mX");
        let rows = screen_rows(parser.screen(), None, Rgb8(255, 255, 255));
        let span = rows[0].iter().find(|s| s.text.contains('X')).unwrap();
        assert_eq!(span.style.fg, None);
        assert_eq!(span.style.bg, Some(Rgb8(205, 0, 0)));
    }

    #[test]
    fn cursor_cell_gets_cursor_background() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"AB");
        // Cursor sits after "AB" → cell (0,2), which is blank.
        let rows = screen_rows(parser.screen(), Some((0, 2)), Rgb8(200, 200, 200));
        let flat: Vec<&TermSpan> = rows[0].iter().collect();
        assert!(flat.iter().any(|s| s.style.bg == Some(Rgb8(200, 200, 200))));
    }

    #[test]
    fn selection_endpoints_normalize_regardless_of_drag_direction() {
        assert_eq!(normalize_selection((2, 9), (0, 1)), ((0, 1), (2, 9)));
        assert_eq!(normalize_selection((1, 3), (1, 7)), ((1, 3), (1, 7)));
        assert_eq!(normalize_selection((1, 7), (1, 3)), ((1, 3), (1, 7)));
    }

    #[test]
    fn point_maps_to_cells_and_clamps_to_grid() {
        // 10px chars, 20px lines, origin at (100, 50), grid 80x24.
        assert_eq!(
            point_to_cell((115.0, 62.0), (100.0, 50.0), 10.0, 20.0, 80, 24),
            (0, 1)
        );
        assert_eq!(
            point_to_cell((100.0, 50.0), (100.0, 50.0), 10.0, 20.0, 80, 24),
            (0, 0)
        );
        // Above/left clamps to origin; far below/right clamps to grid edge
        // (col may be `cols` for an exclusive end-of-line edge).
        assert_eq!(
            point_to_cell((0.0, 0.0), (100.0, 50.0), 10.0, 20.0, 80, 24),
            (0, 0)
        );
        assert_eq!(
            point_to_cell((9999.0, 9999.0), (100.0, 50.0), 10.0, 20.0, 80, 24),
            (23, 80)
        );
        assert_eq!(
            point_to_cell((0.0, 0.0), (0.0, 0.0), 0.0, 20.0, 80, 24),
            (0, 0)
        );
    }

    #[test]
    fn multi_row_selection_splits_into_per_row_spans() {
        assert!(selection_rows((1, 2), (1, 2), 80).is_empty());
        assert_eq!(selection_rows((1, 2), (1, 5), 80), vec![(1, 2, 5)]);
        assert_eq!(
            selection_rows((0, 70), (2, 10), 80),
            vec![(0, 70, 80), (1, 0, 80), (2, 0, 10)]
        );
        // Reverse drag normalizes the same way.
        assert_eq!(
            selection_rows((2, 10), (0, 70), 80),
            vec![(0, 70, 80), (1, 0, 80), (2, 0, 10)]
        );
    }

    #[test]
    fn selected_text_comes_from_emulator_between_cells() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"hello world");
        let text = selection_text(parser.screen(), (0, 0), (0, 5));
        assert_eq!(text, "hello");
        assert!(selection_text(parser.screen(), (0, 3), (0, 3)).is_empty());
        // Reversed endpoints select the same text.
        assert_eq!(selection_text(parser.screen(), (0, 5), (0, 0)), "hello");
    }

    #[test]
    fn colored_span_carries_its_style() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[31mred-text\x1b[0m");
        let rows = screen_rows(parser.screen(), None, Rgb8(255, 255, 255));
        let span = rows[0]
            .iter()
            .find(|s| s.text.contains("red-text"))
            .expect("colored span survives");
        assert_eq!(span.style.fg, Some(Rgb8(205, 0, 0)));
    }
}
