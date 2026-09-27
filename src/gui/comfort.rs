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

    /// Enter folder-picker capture (`w` in nav focus, issue #48): the
    /// buffer starts empty — Enter on empty keeps the default folder.
    pub(crate) fn begin_cwd_capture(&mut self) {
        self.cwd_capture = Some(String::new());
    }

    /// Accept the typed folder and start the session in it (Enter).
    /// Blank means the default folder; a missing folder refuses with a
    /// flash and stays in capture so the typo can be fixed. Returns
    /// true when a session started.
    pub(crate) fn accept_cwd(&mut self) -> bool {
        let raw = self.cwd_capture.clone().unwrap_or_default();
        let picked = crate::app::expand_cwd_input(&raw);
        if let Some(dir) = &picked {
            if !std::path::Path::new(dir).is_dir() {
                self.app.set_status(format!("no such folder: {dir}"));
                return false;
            }
        }
        self.cwd_capture = None;
        self.link_cursor = None;
        if !self.request_new_run_in(picked.clone()) {
            return false;
        }
        if let Some(dir) = picked {
            self.app.set_status(format!("new session in {dir}"));
        }
        true
    }

    /// Cancel folder capture (Esc): no session starts.
    pub(crate) fn cancel_cwd(&mut self) {
        self.cwd_capture = None;
    }

    /// Folder-picker capture dispatch (issue #48). Returns true when the
    /// key was consumed: printable keys extend the path buffer,
    /// Backspace edits, Enter starts the session, Esc cancels (and must
    /// not quit — capture outranks nav keys). `ctrl`/`platform` combos
    /// never count as path text.
    pub(crate) fn cwd_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        ctrl: bool,
        platform: bool,
    ) -> bool {
        if self.cwd_capture.is_none() {
            return false;
        }
        match key {
            "enter" => {
                self.accept_cwd();
            }
            "escape" => {
                self.cancel_cwd();
            }
            "backspace" => {
                if let Some(buf) = self.cwd_capture.as_mut() {
                    buf.pop();
                }
            }
            _ => {
                if !ctrl && !platform {
                    if let Some(c) = key_char {
                        if c.chars().count() == 1 {
                            if let Some(buf) = self.cwd_capture.as_mut() {
                                buf.push_str(c);
                            }
                        }
                    }
                }
            }
        }
        true
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
    fn cwd_capture_starts_a_session_in_the_typed_folder() {
        // Issue #48: `w` opens the picker; typing + Enter starts the
        // session in that folder; Esc cancels with nothing started.
        let mut view = test_shell();
        view.app.focus_nav();
        assert!(view.cwd_capture.is_none());
        view.begin_cwd_capture();
        assert_eq!(view.cwd_capture.as_deref(), Some(""));
        // Capture owns the keyboard: the status line shows the picker.
        assert!(view
            .status_text_for_width(1280.0)
            .contains("new session folder"));
        for c in ["/", "t", "m", "p"] {
            assert!(view.cwd_key(c, Some(c), false, false));
        }
        assert_eq!(view.cwd_capture.as_deref(), Some("/tmp"));
        assert!(view.cwd_key("backspace", None, false, false));
        assert_eq!(view.cwd_capture.as_deref(), Some("/tm"));
        assert!(view.cwd_key("p", Some("p"), false, false));
        // Ctrl combos never count as path text.
        assert!(view.cwd_key("c", Some("c"), true, false));
        assert_eq!(view.cwd_capture.as_deref(), Some("/tmp"));
        assert!(view.cwd_key("enter", None, false, false));
        assert!(view.cwd_capture.is_none());
        let s = view.app.selected_session().unwrap();
        assert_eq!(s.cwd.as_deref(), Some("/tmp"));
        assert!(view.app.has_pending_spawn());
    }

    #[test]
    fn cwd_capture_blank_keeps_default_missing_folder_refuses() {
        // Issue #48: Enter on empty is the plain `n` path; a missing
        // folder flashes and stays in capture for a fix; Esc bails.
        let mut view = test_shell();
        view.app.focus_nav();
        view.begin_cwd_capture();
        assert!(view.cwd_key("enter", None, false, false));
        assert!(view.cwd_capture.is_none());
        assert_eq!(view.app.selected_session().unwrap().cwd, None);

        view.app.focus_nav();
        view.begin_cwd_capture();
        for c in ["/", "n", "o", "-", "s", "u", "c", "h"] {
            assert!(view.cwd_key(c, Some(c), false, false));
        }
        assert!(view.cwd_key("enter", None, false, false));
        // Refused: no session, still capturing, flash names it.
        assert_eq!(view.app.sessions.len(), 1);
        assert!(view.cwd_capture.is_some());
        assert!(
            view.app
                .status_text()
                .is_some_and(|m| m.contains("no such folder")),
            "flash: {:?}",
            view.app.status_text()
        );
        assert!(view.cwd_key("escape", None, false, false));
        assert!(view.cwd_capture.is_none());
        assert_eq!(view.app.sessions.len(), 1);
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
