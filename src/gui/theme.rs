//! Non-color status cues + contrast floor (issue #8).
//!
//! Status is never color-alone: every run row carries a text marker and
//! every pane a text state label (`Muse [live]`, section titles), so the
//! UI survives monochrome and color-vision deficiency. Dim (secondary)
//! text keeps a WCAG contrast ratio >= 4.5 against the dark surfaces.

use crate::app::Status;
use crate::config::{OsAppearance, ThemePreference};

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

/// Sessions-panel surface (issue #52): the panel keeps this dark
/// surface in both theme modes, so one explicit palette covers dark
/// and light — the component theme's translucent label colors (built
/// for a light sidebar) never touch it.
pub(crate) const SIDEBAR_BG: u32 = 0x1e1e2e;
/// Sessions-panel primary text: rows and the interactive History
/// toggle. 11.07 on [`SIDEBAR_BG`].
pub(crate) const SIDEBAR_FG: u32 = 0xd4d4d4;
/// Sessions-panel section-header text (issue #52): dimmer than rows
/// for the Finder-style hierarchy, still 7.06 on [`SIDEBAR_BG`] —
/// the component's 70%-opacity theme label it replaces drops to 1.07
/// in light mode (dark `#171717` text on this dark surface).
pub(crate) const SIDEBAR_HEADER_FG: u32 = 0xAAAAAA;

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

/// Map the configured theme choice plus the live gpui window appearance
/// onto the component theme mode the chrome reads. This is the only place
/// gpui's `WindowAppearance` meets `config`: both `main` (startup) and
/// `shell` (the `t`-key cycle) funnel through here, so a gpui upgrade
/// touches one function. Native shells do the same mapping against
/// [`OsAppearance`] on their side (see `docs/native-core-seam.md`).
pub(crate) fn theme_mode_for(
    pref: ThemePreference,
    appearance: gpui::WindowAppearance,
) -> gpui_component::ThemeMode {
    let os = match appearance {
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => OsAppearance::Dark,
        gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => OsAppearance::Light,
    };
    // Dogfood the framework-free seam (`resolve`/`is_dark`) native shells
    // will bind against, so it stays exercised outside tests.
    if pref.is_dark(os) {
        gpui_component::ThemeMode::Dark
    } else {
        gpui_component::ThemeMode::Light
    }
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
    fn sidebar_text_clears_floor_in_both_modes() {
        // Issue #52: the panel keeps SIDEBAR_BG in both theme modes,
        // so these explicit colors are the whole story — rows/toggle
        // at 11.07, section headers at 7.06. The component label they
        // replace (`sidebar_foreground` at 70% over this surface)
        // measures 7.94 in dark mode but 1.07 in light mode.
        for (name, fg) in [
            ("row/toggle", SIDEBAR_FG),
            ("section header", SIDEBAR_HEADER_FG),
        ] {
            let ratio = contrast_ratio(fg, SIDEBAR_BG);
            assert!(
                ratio >= 4.5,
                "sidebar {name} text too dim: {ratio:.2} (measured)"
            );
        }
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

    #[test]
    fn vibrant_appearances_resolve_to_their_base_mode() {
        // The gpui adapter preserves the old `theme_mode` contract:
        // vibrant variants follow their base, explicit choices ignore
        // the OS, and system-on-unknown is covered by the core test.
        use gpui::WindowAppearance;
        use gpui_component::ThemeMode;
        assert_eq!(
            super::theme_mode_for(ThemePreference::System, WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
        assert_eq!(
            super::theme_mode_for(ThemePreference::System, WindowAppearance::VibrantLight),
            ThemeMode::Light
        );
        assert_eq!(
            super::theme_mode_for(ThemePreference::Dark, WindowAppearance::Light),
            ThemeMode::Dark
        );
        assert_eq!(
            super::theme_mode_for(ThemePreference::Light, WindowAppearance::Dark),
            ThemeMode::Light
        );
    }
}
