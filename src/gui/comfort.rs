//! Local-only comfort UX: font/panel keys + title filter (issue #29).
//!
//! `+`/`-` resize the terminal font, `[`/`]` resize the sessions panel —
//! both persist to local JSON prefs so they survive restarts. `/` opens a
//! title-substring filter that hides non-matching runs from the panel
//! without changing sort order. All dispatch here is window-free
//! (`filter_key`, `comfort_key`) so headless tests cover it; `on_key`
//! only forwards.

use super::shell::ShellView;
#[cfg(not(test))]
use crate::prefs::Prefs;

impl ShellView {
    /// Persist the current comfort settings (best effort: a failed save
    /// just falls back to defaults next start). Skipped in unit-test
    /// builds: dispatch and clamping are covered here while the
    /// save/load roundtrip is covered in `prefs.rs`, so `cargo test`
    /// never rewrites the developer's live prefs file.
    fn persist_comfort(&self) {
        #[cfg(not(test))]
        let _ = Prefs {
            font_size: self.font_size,
            sidebar_width: self.sidebar_width,
        }
        .save();
    }

    /// Grow (`delta > 0`) or shrink the terminal font, clamped and
    /// persisted, with a confirming flash.
    pub(crate) fn adjust_font(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(8.0, 32.0);
        // Cell metrics depend on the size: drop the one-time cache so the
        // next frame re-measures and the PTY grid follows the new font.
        self.mono_metrics = None;
        self.persist_comfort();
        self.app.set_status(format!("font {:.0}pt", self.font_size));
    }

    /// Widen (`delta > 0`) or narrow the sessions panel, clamped and
    /// persisted, with a confirming flash.
    pub(crate) fn adjust_sidebar(&mut self, delta: f32) {
        self.sidebar_width = (self.sidebar_width + delta).clamp(160.0, 480.0);
        self.persist_comfort();
        self.app
            .set_status(format!("panel {:.0}px", self.sidebar_width));
    }

    /// Comfort keys in nav focus. Returns true when consumed: `+`/`-`
    /// font, `[`/`]` panel. Layout-correct via `key_char` (the produced
    /// character), never firing in terminal focus where they must type.
    pub(crate) fn comfort_key(&mut self, key_char: Option<&str>) -> bool {
        match key_char {
            Some("+") | Some("=") => {
                self.adjust_font(1.0);
                true
            }
            Some("-") | Some("_") => {
                self.adjust_font(-1.0);
                true
            }
            Some("]") => {
                self.adjust_sidebar(24.0);
                true
            }
            Some("[") => {
                self.adjust_sidebar(-24.0);
                true
            }
            _ => false,
        }
    }

    /// Enter filter capture (`/` when idle).
    pub(crate) fn begin_filter(&mut self) {
        self.filtering = true;
    }

    /// Append to the live filter, snapping selection into the matches.
    pub(crate) fn filter_type(&mut self, text: &str) {
        let mut next = self.app.filter.clone();
        next.push_str(text);
        self.apply_filter_text(next);
    }

    /// Accept the filter and leave capture (Enter): the panel stays
    /// filtered until `/`, Esc clears it.
    pub(crate) fn accept_filter(&mut self) {
        self.filtering = false;
        if self.app.filter.is_empty() {
            self.app.set_status("filter cleared");
        } else {
            let n = self.app.visible_indices().len();
            self.app
                .set_status(format!("filter '{}' · {n} runs", self.app.filter));
        }
    }

    /// Cancel capture (Esc): exits capture and clears the filter.
    pub(crate) fn cancel_filter(&mut self) {
        self.filtering = false;
        self.apply_filter_text(String::new());
        self.app.set_status("filter cleared");
    }

    fn apply_filter_text(&mut self, text: String) {
        self.app.set_filter(text);
        self.link_cursor = None;
    }

    /// Filter capture dispatch for nav focus. Returns true when the key
    /// was consumed: `/` opens capture; while capturing, printable keys
    /// extend the filter, Backspace edits, Enter accepts, Esc cancels
    /// (and must not quit — capture outranks nav keys). `ctrl` combos
    /// never count as filter text.
    pub(crate) fn filter_key(&mut self, key: &str, key_char: Option<&str>, ctrl: bool) -> bool {
        if !self.filtering {
            if !ctrl && (key == "/" || key_char == Some("/")) {
                self.begin_filter();
                return true;
            }
            return false;
        }
        match key {
            "enter" => {
                self.accept_filter();
            }
            "escape" => {
                self.cancel_filter();
            }
            "backspace" => {
                let mut next = self.app.filter.clone();
                next.pop();
                self.apply_filter_text(next);
            }
            _ => {
                if !ctrl {
                    if let Some(c) = key_char {
                        if c.chars().count() == 1 {
                            self.filter_type(c);
                        }
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::runs::test_shell;
    use super::*;

    fn fox_shell() -> ShellView {
        let mut view = test_shell();
        for title in ["fox hunt", "otter den", "Fox trot"] {
            view.app.start_new_session();
            let _ = view.app.take_pending_spawn();
            let id = view.active_id().unwrap();
            let s = view.app.sessions.iter_mut().find(|s| s.id == id).unwrap();
            s.title = title.to_string();
            s.title_locked = true;
            view.app.focus_nav();
        }
        view.app.selected = 0;
        view
    }

    #[test]
    fn slash_filter_shows_only_matching_titles_unsorted() {
        let mut view = fox_shell();
        let before: Vec<String> = view.app.sessions.iter().map(|s| s.id.clone()).collect();
        assert!(view.filter_key("/", Some("/"), false));
        assert!(view.filtering);
        for c in ["f", "o", "x"] {
            assert!(view.filter_key(c, Some(c), false));
        }
        assert_eq!(view.app.filter, "fox");
        let visible: Vec<String> = view
            .app
            .visible_indices()
            .iter()
            .map(|&i| view.app.sessions[i].title.clone())
            .collect();
        assert_eq!(visible, vec!["fox hunt", "Fox trot"]);
        // Sort order untouched: ids keep their relative order.
        let after: Vec<String> = view.app.sessions.iter().map(|s| s.id.clone()).collect();
        assert_eq!(before, after);
        // Selection snapped into the matches.
        let sel = view.app.selected_session().unwrap().title.clone();
        assert!(sel.to_lowercase().contains("fox"), "selected {sel:?}");
        // Enter accepts and leaves capture; the panel stays filtered.
        assert!(view.filter_key("enter", None, false));
        assert!(!view.filtering);
        assert_eq!(view.app.filter, "fox");
        // `/` then Esc clears the filter entirely.
        assert!(view.filter_key("/", Some("/"), false));
        assert!(view.filter_key("escape", None, false));
        assert!(!view.filtering);
        assert!(view.app.filter.is_empty());
        assert_eq!(view.app.visible_indices().len(), 3);
    }

    #[test]
    fn comfort_keys_resize_and_persist_without_reaching_the_pty() {
        let mut view = test_shell();
        let base_font = view.font_size;
        let base_panel = view.sidebar_width;
        assert!(view.comfort_key(Some("+")));
        assert_eq!(view.font_size, base_font + 1.0);
        assert!(view.comfort_key(Some("-")));
        assert_eq!(view.font_size, base_font);
        assert!(view.comfort_key(Some("]")));
        assert_eq!(view.sidebar_width, base_panel + 24.0);
        assert!(view.comfort_key(Some("[")));
        assert_eq!(view.sidebar_width, base_panel);
        assert!(!view.comfort_key(Some("q")));
        // Clamp guards: no runaway growth or collapse.
        for _ in 0..100 {
            view.comfort_key(Some("+"));
        }
        assert!(view.font_size <= 32.0);
        for _ in 0..100 {
            view.comfort_key(Some("["));
        }
        assert!(view.sidebar_width >= 160.0);
    }
}
