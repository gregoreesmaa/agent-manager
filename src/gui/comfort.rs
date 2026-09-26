//! Local-only comfort UX: font/panel keys + title filter (issue #29).
//!
//! `+`/`-` resize the terminal font, `[`/`]` resize the sessions panel —
//! both persist to the local JSON config file (shared with the #33–#35
//! settings) so they survive restarts. `/` opens a
//! title-substring filter that hides non-matching runs from the panel
//! without changing sort order. All dispatch here is window-free
//! (`filter_key`, `comfort_key`) so headless tests cover it; `on_key`
//! only forwards.

use super::shell::ShellView;

impl ShellView {
    /// Persist the comfort settings (best effort: a failed save just
    /// falls back to file defaults next start). Skipped in unit-test
    /// builds: dispatch and clamping are covered here while the
    /// save/load roundtrip is covered in `config.rs`, so `cargo test`
    /// never rewrites the developer's live config file.
    fn persist_comfort(&self) {
        #[cfg(not(test))]
        let _ = self.app.save_config();
    }

    /// Grow (`delta > 0`) or shrink the terminal font, clamped and
    /// persisted, with a confirming flash. The metrics cache is keyed by
    /// the configured size, so the next frame re-measures on its own.
    pub(crate) fn adjust_font(&mut self, delta: f32) {
        let size = self.app.terminal_config().font_size + delta;
        let size = self.app.set_terminal_font_size(size);
        self.persist_comfort();
        self.app.set_status(format!("font {size:.0}pt"));
    }

    /// Widen (`delta > 0`) or narrow the sessions panel, clamped and
    /// persisted, with a confirming flash.
    pub(crate) fn adjust_sidebar(&mut self, delta: f32) {
        let width = self.app.sidebar_width() + delta;
        let width = self.app.set_sidebar_width(width);
        self.persist_comfort();
        self.app.set_status(format!("panel {width:.0}px"));
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
        let base_font = view.app.terminal_config().font_size;
        let base_panel = view.app.sidebar_width();
        assert!(view.comfort_key(Some("+")));
        assert_eq!(view.app.terminal_config().font_size, base_font + 1.0);
        assert!(view.comfort_key(Some("-")));
        assert_eq!(view.app.terminal_config().font_size, base_font);
        assert!(view.comfort_key(Some("]")));
        assert_eq!(view.app.sidebar_width(), base_panel + 24.0);
        assert!(view.comfort_key(Some("[")));
        assert_eq!(view.app.sidebar_width(), base_panel);
        assert!(!view.comfort_key(Some("q")));
        // Clamp guards: no runaway growth or collapse.
        for _ in 0..100 {
            view.comfort_key(Some("+"));
        }
        assert!(view.app.terminal_config().font_size <= 32.0);
        for _ in 0..100 {
            view.comfort_key(Some("["));
        }
        assert!(view.app.sidebar_width() >= 160.0);
    }
}
