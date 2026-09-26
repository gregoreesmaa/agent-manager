//! Responsive layout geometry (issue #6).
//!
//! Pure, framework-free math: the fixed 264px sidebar + 28px bar broke
//! below ~700px while `fit_pty` floored at 200x120. Below the narrow
//! breakpoint the sidebar collapses (terminal takes the full width),
//! the window minimum matches the PTY floors, and status hints compact.

/// Left runs panel width in pixels (consumed only when visible).
pub(crate) const LEFT_WIDTH: f32 = 264.0;
/// Status bar height in pixels. Only rendered in narrow mode (issue
/// #32): wide mode hosts the status text in the sidebar footer.
pub(crate) const STATUS_HEIGHT: f32 = 28.0;
/// Viewport width below which the runs sidebar collapses so the terminal
/// pane gets the full width.
pub const NARROW_BREAKPOINT: f32 = 700.0;
/// PTY grid floors: the smallest live grid `fit_pty` will ever report.
/// Shared with the window minimum size so the OS never lets the window
/// shrink past what the terminal can display.
pub const MIN_COLS: u16 = 20;
pub const MIN_ROWS: u16 = 10;
/// Fallback monospace metrics (same fallbacks as `mono_metrics`); used to
/// derive the static window minimum below.
pub(crate) const FALLBACK_CHAR_W: f32 = 8.0;
pub(crate) const FALLBACK_LINE_H: f32 = 18.0;
/// Minimum window size, derived from the PTY floors: wide enough for
/// MIN_COLS beside the sidebar at fallback metrics, tall enough for
/// MIN_ROWS plus the narrow-mode status bar (issue #32 removed the
/// terminal header, so wide mode has no vertical chrome at all).
/// (424 x 208).
pub const MIN_WINDOW_WIDTH: f32 = LEFT_WIDTH + MIN_COLS as f32 * FALLBACK_CHAR_W;
pub const MIN_WINDOW_HEIGHT: f32 = MIN_ROWS as f32 * FALLBACK_LINE_H + STATUS_HEIGHT;

/// Vertical chrome above/below the terminal pane at this viewport width
/// (issue #32): zero in wide mode (status lives in the sidebar footer),
/// the slim status bar height when the sidebar is collapsed.
pub fn chrome_height_for(viewport_w: f32) -> f32 {
    if sidebar_visible_for_width(viewport_w) {
        0.0
    } else {
        STATUS_HEIGHT
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_viewport_collapses_sidebar_and_frees_pty_width() {
        // Issue #6: below ~700px the 264px sidebar collapses so the
        // terminal pane gets the full width.
        assert!(!sidebar_visible_for_width(699.0));
        assert!(sidebar_visible_for_width(700.0));
        assert!(sidebar_visible_for_width(1280.0));
        assert_eq!(effective_sidebar_width(699.0), 0.0);
        assert_eq!(effective_sidebar_width(800.0), LEFT_WIDTH);
        // A 600px narrow window gives the PTY the full 600px (75 cols at
        // 8px) instead of 600-264=336px (42 cols) beside the sidebar.
        let (collapsed_cols, _) = pty_grid_for(600.0, 400.0, 8.0, 18.0);
        let (beside_cols, _) = pty_grid_for(600.0 - LEFT_WIDTH, 400.0, 8.0, 18.0);
        assert_eq!(collapsed_cols, 75);
        assert_eq!(beside_cols, 42);
        assert!(collapsed_cols > beside_cols);
    }

    #[test]
    fn wide_mode_has_no_vertical_chrome_narrow_keeps_the_bar() {
        // Issue #32: the terminal owns the full height in wide mode;
        // collapsed (narrow) mode keeps the 28px status bar.
        assert_eq!(chrome_height_for(1280.0), 0.0);
        assert_eq!(chrome_height_for(700.0), 0.0);
        assert_eq!(chrome_height_for(699.0), STATUS_HEIGHT);
        // The window minimum still fits a full grid at fallback metrics
        // in the worst (narrow) case.
        let (cols, rows) = pty_grid_for(
            MIN_WINDOW_WIDTH - LEFT_WIDTH,
            MIN_WINDOW_HEIGHT - STATUS_HEIGHT,
            FALLBACK_CHAR_W,
            FALLBACK_LINE_H,
        );
        assert!(cols >= MIN_COLS, "min width fits {cols} cols");
        assert!(rows >= MIN_ROWS, "min height fits {rows} rows");
    }

    #[test]
    fn pty_grid_floors_and_ceilings_match_window_minimum() {
        // Degenerate sizes still report a usable grid (cols>=20/rows>=10):
        // the 200x120px floors divide to 25x6 at 8x18 metrics, and the row
        // clamp lifts 6 to the MIN_ROWS floor of 10.
        assert_eq!(pty_grid_for(0.0, 0.0, 8.0, 18.0), (25, 10));
        assert_eq!(pty_grid_for(-50.0, -50.0, 8.0, 18.0), (25, 10));
        // Huge windows clamp instead of overflowing the u16 grid.
        assert_eq!(pty_grid_for(100_000.0, 100_000.0, 8.0, 18.0), (400, 200));
        // Ordinary sizes divide exactly.
        assert_eq!(pty_grid_for(800.0, 360.0, 8.0, 18.0), (100, 20));
    }
}
