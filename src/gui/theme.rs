//! Non-color status cues + contrast floor (issue #8).
//!
//! Status is never color-alone: every run row carries a text marker and
//! every pane a text state label (`Muse [live]`, section titles), so the
//! UI survives monochrome and color-vision deficiency. Dim (secondary)
//! text keeps a WCAG contrast ratio >= 4.5 against the dark surfaces.

use crate::app::Status;

/// Dim secondary foreground. 0xAAAAAA on the 0x11111b terminal surface
/// (and the 0x1e1e2e bars) clears the 4.5 floor the old 0x888888 missed;
/// see [`secondary_contrast_clears_floor`].
pub(crate) const SECONDARY_FG: u32 = 0xAAAAAA;
/// Terminal surface background (contrast-floor reference only).
#[cfg(test)]
pub(crate) const SURFACE_BG: u32 = 0x11111b;
/// Bar background (status bar, help panel; contrast-floor reference only).
#[cfg(test)]
pub(crate) const BAR_BG: u32 = 0x1e1e2e;

/// Row marker per status: always non-blank, Attention distinct from the
/// rest, so runs are distinguishable with color removed.
pub(crate) fn row_marker(status: Status) -> &'static str {
    match status {
        Status::Attention => "!",
        Status::Idle => "·",
        Status::Working => ">",
    }
}

/// Section-group marker per status (same vocabulary as [`row_marker`]).
pub(crate) fn group_marker(status: Status) -> &'static str {
    row_marker(status)
}

/// WCAG 2.x contrast ratio of two 0xRRGGBB colors (1.0 – 21.0).
/// Test-only until the theme module grows runtime contrast checks (#34).
#[cfg(test)]
pub(crate) fn contrast_ratio(fg: u32, bg: u32) -> f64 {
    fn luminance(c: u32) -> f64 {
        let channel = |v: u32| {
            let s = (v & 0xFF) as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c >> 16) + 0.7152 * channel(c >> 8) + 0.0722 * channel(c)
    }
    let (l1, l2) = (luminance(fg), luminance(bg));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_contrast_clears_floor() {
        // Issue #8: dim text must keep ratio >= 4.5 on both dark
        // surfaces, with headroom (8.07 / 7.06, independently checked).
        assert!(contrast_ratio(SECONDARY_FG, SURFACE_BG) >= 4.5);
        assert!(contrast_ratio(SECONDARY_FG, BAR_BG) >= 4.5);
    }

    #[test]
    fn markers_stay_non_blank_and_distinct() {
        for status in [Status::Attention, Status::Idle, Status::Working] {
            assert!(!row_marker(status).is_empty());
            assert!(!group_marker(status).is_empty());
        }
        assert_ne!(row_marker(Status::Attention), row_marker(Status::Idle));
        assert_ne!(row_marker(Status::Attention), row_marker(Status::Working));
    }
}
