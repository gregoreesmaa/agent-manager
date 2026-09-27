//! Sessions-panel render for [`super::shell::ShellView`].
//!
//! Plain-gpui chrome: a pinned header (`Sessions` + `+ New` + the visible
//! search field), one group per live status section (Needs input / Idle /
//! Active) plus a collapsible History group, session rows (each with its
//! folder tail when the session picked one, issue #48), and every
//! accumulated parsed link as a child row — display-capped with an
//! `N more` disclosure over fully retained data. Clicking a link opens
//! it in the default browser and copies it; modifier-click copies only
//! (issue #49). Keyboard link focus (`o`/Enter/`y`) highlights the
//! focused row via `active`. The header doubles as the window drag
//! region now that the OS title bar is hidden (issue #44).
//!
//! Layout contract (issues #39/#40/#45/#50):
//!
//! - The list column owns its scroll container (`overflow_y_scroll` over
//!   a natural-height child, so overflow always scrolls) with a drag
//!   scrollbar bound to the shared panel handle; the header and the
//!   footer are siblings of the scroller, so both stay pinned while
//!   the list moves. Wheel/trackpad scroll needs no handle at all.
//! - History holds exited live runs plus historic provider entries with
//!   no live PTY ([`ShellView::is_history`]); it renders collapsed by
//!   default with its own count, toggled by click or `h`.
//! - The search field (issue #45) is a clickable row wired to the
//!   existing `App::filter`/`matches_filter` capture: `/` focuses it,
//!   typing filters live, the row shows the match count plus a clear
//!   affordance, Esc clears. The footer no longer duplicates the filter.
//! - The static `Sessions` label is a plain div (issue #50): zero hover
//!   affordance, while `+ New` and the rows keep their own hover.
//! - Every row starts with its harness badge (issue #46), ahead of the
//!   status row marker.

use gpui::{
    div, px, rgb, AnyElement, ClickEvent, Context, ElementId, InteractiveElement,
    IntoElement, ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, WindowControlArea,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    scroll::Scrollbar,
    sidebar::{SidebarGroup, SidebarMenu, SidebarMenuItem},
    Sizable as _,
};

use crate::app::{section_title, status_sections};

use super::shell::ShellView;

/// Panel chrome surface: the window root is hardcoded dark (see `view`),
/// so the panel matches with the same bar surface instead of resolving
/// the component theme per frame.
const PANEL_BG: u32 = 0x1e1e2e;
/// Primary panel text (the default foreground the terminal pane uses).
const PANEL_FG: u32 = 0xd4d4d4;

/// Scroll state for the session list (issue #40): the process owns one
/// main window, so one shared handle binds the list viewport to its
/// drag scrollbar. Wheel/trackpad scrolling works without it; the drag
/// thumb and any future follow-selection read this same handle.
fn panel_scroll() -> ScrollHandle {
    // Thread-local: `ScrollHandle` is `!Send + !Sync` (Rc<RefCell>
    // state), and gpui renders on the main thread, so one handle per
    // process is exactly one handle for the one main window.
    std::thread_local! {
        static HANDLE: std::cell::RefCell<Option<ScrollHandle>> =
            const { std::cell::RefCell::new(None) };
    }
    HANDLE.with(|h| {
        let mut h = h.borrow_mut();
        if h.is_none() {
            *h = Some(ScrollHandle::new());
        }
        h.clone().expect("just initialized")
    })
}

impl ShellView {
    /// History membership (issue #39): exited live runs plus historic
    /// provider entries with no live PTY. Fresh runs awaiting spawn (no
    /// PTY yet, no provider id) and live children stay in the active
    /// groups; re-attaching a historic entry (live PTY again) returns it
    /// to the active groups until its child exits.
    pub(crate) fn is_history(&self, session: &crate::app::ChatSession) -> bool {
        match self.runs.get(&session.id) {
            Some(run) => run.exited(),
            None => session.provider_session_id.is_some(),
        }
    }

    /// Indices of history entries in list order. Filter-independent:
    /// callers apply [`crate::app::App::matches_filter`] so emptied
    /// History drops out exactly like the live groups.
    pub(crate) fn history_indices(&self) -> Vec<usize> {
        self.app
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| self.is_history(s))
            .map(|(i, _)| i)
            .collect()
    }

    /// Sessions panel: a pinned header, the scrolling session list with
    /// its drag scrollbar, and a pinned footer (issue #32 keeps the
    /// status line here so the terminal owns every vertical pixel).
    /// The empty-state line rides in the footer while there are no
    /// sessions. The terminal pane stays hand-rolled gpui.
    pub(crate) fn sidebar_footer(&self, viewport_w: f32) -> gpui::Div {
        // Filter state (issues #29/#45) lives in the header search
        // field now: the footer never duplicates it.
        let mut footer = div().flex().flex_col().w_full().px_3().pb_3().pt_1();
        if self.app.sessions.is_empty() {
            footer = footer.child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .text_xs()
                    .child("No sessions yet.".to_string()),
            );
        }
        // Folder detail (issue #48): the selected run's full working
        // directory — the row itself shows only the glanceable tail.
        if let Some(dir) = self.app.selected_session().and_then(|s| s.cwd.as_deref()) {
            footer = footer.child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .text_xs()
                    .truncate()
                    .child(format!("in {dir}")),
            );
        }
        footer.child(
            div()
                .text_color(rgb(super::theme::SECONDARY_FG))
                .text_xs()
                .truncate()
                .child(self.status_text_for_width(viewport_w)),
        )
    }

    /// Pinned panel header: the static `Sessions` label (a plain div, so
    /// hovering it never hints at an interaction — issue #50) beside the
    /// real `+ New` button, with the visible search field underneath
    /// (issue #45).
    fn panel_header(&self, cx: &mut Context<Self>) -> gpui::Div {
        // Issue #44: with the OS title bar hidden the header top row is
        // the window drag region (traffic lights float over the
        // content). Child controls keep their own hitboxes, so `+ New`
        // still clicks while the bare header drags the window.
        let mut header = div().flex().flex_col().w_full().child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .w_full()
                .px_2()
                .pt_2()
                .window_control_area(WindowControlArea::Drag)
                .child(div().child("Sessions".to_string()))
                .child(
                    Button::new(ElementId::Name("new-run-btn".into()))
                        .label("+ New")
                        .primary()
                        .small()
                        .on_click(cx.listener(|this, _ev, window, _cx| {
                            // Same live-run cap as the `n` key (issue #31):
                            // a refused 11th run never steals focus.
                            if this.request_new_run() {
                                this.focus_term(window);
                            }
                        })),
                ),
        );
        if !self.app.sessions.is_empty() {
            header = header.child(self.search_row(cx));
        }
        header
    }

    /// Visible search field (issue #45): a row wired to the existing
    /// `App::filter`/`matches_filter` capture. Clicking it focuses the
    /// filter exactly like `/`; typing filters live; the row shows the
    /// match count plus a clear affordance; Esc clears and returns focus
    /// as today. Keyboard parity holds by construction: the row renders
    /// capture state, it never reimplements it.
    fn search_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = div()
            .id(ElementId::Name("sidebar-search".into()))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .px_2()
            .pb_2()
            .on_click(cx.listener(|this, _ev, _window, _cx| {
                this.begin_filter();
            }));
        if self.app.filter.is_empty() {
            row.child(
                div()
                    .flex_1()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .child(
                        if self.filtering {
                            "❯ Filter…"
                        } else {
                            "⌕ Filter…  ( / )"
                        }
                        .to_string(),
                    ),
            )
            .into_any_element()
        } else {
            let shown = self.app.visible_indices().len();
            let total = self.app.sessions.len();
            row.child(
                div()
                    .flex_1()
                    .truncate()
                    .child(format!("⌕ {}", self.app.filter)),
            )
            .child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .text_xs()
                    .child(format!("{shown}/{total}")),
            )
            .child(
                div()
                    .id(ElementId::Name("sidebar-search-clear".into()))
                    .px_1()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        // The clear button sits inside the focusing row:
                        // stop here so the click clears without
                        // re-entering capture.
                        cx.stop_propagation();
                        this.cancel_filter();
                    }))
                    .child("✕".to_string()),
            )
            .into_any_element()
        }
    }

    /// One session row: the harness badge (issue #46) ahead of the
    /// status row marker, then the title plus the session's folder tail
    /// when it picked one (issue #48). Parsed links ride as child items:
    /// click opens the URL and copies it, modifier-click copies only
    /// (issue #49); display-capped with the first MAX_VISIBLE_LINKS live
    /// and the rest folded behind an N more disclosure, fully retained
    /// (and copyable via yank) in the session. `o` focuses, Enter opens,
    /// `y` copies from the keyboard when a row holds link focus
    /// (highlighted via `active`). Non-blank idle marker: rows stay
    /// distinguishable with color removed (see theme::row_marker); the
    /// harness short tag does the same job for the harness half of the
    /// prefix.
    fn session_row(&self, cx: &mut Context<Self>, i: usize) -> SidebarMenuItem {
        let s = &self.app.sessions[i];
        // Harness badge (issue #46): glyph + short tag at the row start,
        // mapped from the harness id with a generic fallback for
        // unknown ids. Group headers and row markers are untouched.
        let (glyph, short) = crate::app::harness_badge(&s.harness);
        let row_marker = super::theme::row_marker(s.status);
        let row_id = s.id.clone();
        let short_of = |link: &str| {
            link.trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("github.com/")
                .to_string()
        };
        // Parsed links as child items: click opens the URL and copies
        // it, modifier-click copies only (issue #49).
        let mut links: Vec<SidebarMenuItem> = Vec::new();
        let (shown_prs, hidden_prs) = crate::app::visible_links(&s.pr_links);
        for (li, link) in shown_prs.iter().enumerate() {
            let short = short_of(link);
            let url = link.clone();
            let focused = i == self.app.selected && self.link_cursor == Some(li);
            // Issue #49: plain click opens the exact URL in the default
            // browser and copies it; Cmd/Ctrl/Shift-click copies without
            // opening.
            links.push(
                SidebarMenuItem::new(short)
                    .active(focused)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _window, cx| {
                        this.link_cursor = Some(li);
                        let m = ev.modifiers();
                        let copy_only = m.platform || m.control || m.shift;
                        this.activate_link(&url, true, copy_only, cx);
                    })),
            );
        }
        let (shown_rel, hidden_rel) = crate::app::visible_links(&s.related_links);
        // Related (issue/commit/file) links share the same open/copy
        // treatment; their link-cursor indices follow the PR rows so
        // `o`/Enter reaches them too.
        let pr_count = s.pr_links.len();
        for (ri, link) in shown_rel.iter().enumerate() {
            let short = short_of(link);
            let url = link.clone();
            let cursor = pr_count + ri;
            let focused = i == self.app.selected && self.link_cursor == Some(cursor);
            links.push(
                SidebarMenuItem::new(short)
                    .active(focused)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _window, cx| {
                        this.link_cursor = Some(cursor);
                        let m = ev.modifiers();
                        let copy_only = m.platform || m.control || m.shift;
                        this.activate_link(&url, false, copy_only, cx);
                    })),
            );
        }
        let hidden = hidden_prs + hidden_rel;
        if hidden > 0 {
            links.push(SidebarMenuItem::new(format!("… {hidden} more")));
        }
        if s.links_truncated {
            links.push(SidebarMenuItem::new("(capped at 50 per list)".to_string()));
        }
        // Issue #48: the session's folder tail rides the row so
        // multi-folder work is glanceable; the full path lives in the
        // footer detail line.
        let row_label = match s.cwd.as_deref() {
            Some(dir) => format!(
                "{glyph} {short} {row_marker} {} · {}",
                s.title,
                crate::app::short_cwd(dir)
            ),
            None => format!("{glyph} {short} {row_marker} {}", s.title),
        };
        SidebarMenuItem::new(row_label)
            .active(i == self.app.selected)
            .default_open(true)
            .on_click(cx.listener(move |this, _ev, window, _cx| {
                if let Some(pos) = this.app.sessions.iter().position(|s| s.id == row_id) {
                    this.app.selected = pos;
                }
                this.clear_selection();
                this.link_cursor = None;
                this.focus_list(window);
            }))
            .children(links)
    }

    pub(crate) fn render_runs(&self, cx: &mut Context<Self>, viewport_w: f32) -> AnyElement {
        // Header with a real button: sessions start here, not at a key hint.
        // The footer carries the status line (issue #32), so the app name
        // and hints live in the panel, not in their own bar.
        // Width follows the persisted comfort setting (issue #29).
        let panel = div()
            .flex()
            .flex_col()
            .h_full()
            .w(px(self.app.sidebar_width()))
            .bg(rgb(PANEL_BG))
            .text_color(rgb(PANEL_FG))
            .child(self.panel_header(cx));
        if self.app.sessions.is_empty() {
            // No list to scroll: a spacer keeps the footer pinned to the
            // bottom exactly like the scroll column does below.
            return panel
                .child(div().flex_1())
                .child(self.sidebar_footer(viewport_w))
                .into_any_element();
        }
        let history = self.history_indices();
        // Live groups only: history entries never mix into Needs input /
        // Idle / Active. Title filter (issue #29): non-matching runs hide
        // without changing sort order; emptied groups drop out entirely.
        let mut list = div().flex().flex_col().w_full().p_3().gap_3();
        for (status, indices) in status_sections(&self.app.sessions) {
            let marker = super::theme::group_marker(status);
            let items: Vec<SidebarMenuItem> = indices
                .into_iter()
                .filter(|i| !history.contains(i) && self.app.matches_filter(&self.app.sessions[*i]))
                .map(|i| self.session_row(cx, i))
                .collect();
            if items.is_empty() {
                continue;
            }
            let group = SidebarGroup::new(format!(
                "{marker} {} ({})",
                section_title(status),
                items.len()
            ))
            .child(SidebarMenu::new().children(items));
            list = list.child(group);
        }
        // History (issue #39): its own count so nothing looks silently
        // dropped, collapsed by default, expanded by click or `h`. Rows
        // are siblings of the toggle (never a submenu), so no internal
        // open-state can desync from `history_expanded`. The toggle row
        // highlights while the selection sits inside collapsed History,
        // so a hidden selection still has a visible anchor.
        let matching_history: Vec<usize> = history
            .into_iter()
            .filter(|i| self.app.matches_filter(&self.app.sessions[*i]))
            .collect();
        if !matching_history.is_empty() {
            let expanded = self.app.history_expanded;
            let selected_inside = matching_history.contains(&self.app.selected);
            let mut items = vec![SidebarMenuItem::new(if expanded {
                "▾ Hide history"
            } else {
                "▸ Show history"
            })
            .active(selected_inside)
            .on_click(cx.listener(|this, _ev, _window, _cx| {
                this.app.toggle_history();
            }))];
            if expanded {
                items.extend(matching_history.iter().map(|i| self.session_row(cx, *i)));
            }
            list = list.child(
                SidebarGroup::new(format!("History ({})", matching_history.len()))
                    .child(SidebarMenu::new().children(items)),
            );
        }
        panel
            .child(
                // The scroll container (issue #40): a bounded-height row
                // whose list child keeps its natural height, so overflow
                // always scrolls (wheel/trackpad), with the drag
                // scrollbar bound to the shared panel handle. Header and
                // footer are siblings of this row, so both stay pinned.
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id(ElementId::Name("sessions-scroll".into()))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&panel_scroll())
                            .child(list),
                    )
                    .child(Scrollbar::vertical(&panel_scroll())),
            )
            .child(self.sidebar_footer(viewport_w))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::runs::test_shell;
    use super::*;
    use crate::app::{ChatSession, Status};

    fn sess(id: &str, status: Status, last_active: i64, provider: Option<&str>) -> ChatSession {
        ChatSession {
            id: id.into(),
            title: id.into(),
            project: "proj".into(),
            status,
            harness: crate::app::HARNESS_MUSE.into(),
            last_active,
            pr_links: vec![],
            related_links: vec![],
            links_truncated: false,
            transcript: vec![],
            transcript_truncated: false,
            provider_session_id: provider.map(|s| s.into()),
            title_locked: true,
            pending_input: String::new(),
        }
    }

    fn mixed_shell() -> ShellView {
        // Sorted by App::new (Idle by recency, then Working): index 0 is
        // the live idle run, 1 the historic entry, 2 the live working run.
        let mut view = ShellView::new_with_sessions(vec![
            sess("live-working", Status::Working, 1, None),
            sess("live-idle", Status::Idle, 3, None),
            sess("hist", Status::Idle, 2, Some("sess-abc")),
        ]);
        assert_eq!(view.app.sessions[0].id, "live-idle");
        assert_eq!(view.app.sessions[1].id, "hist");
        assert_eq!(view.app.sessions[2].id, "live-working");
        // The working run owns a live PTY; the idle run is fresh (spawn
        // pending, no PTY yet); the historic entry has no live PTY.
        let live_id = view
            .app
            .sessions
            .iter()
            .find(|s| s.id == "live-working")
            .unwrap()
            .id
            .clone();
        let pty = crate::embedded::EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.runs.insert(live_id, crate::gui::runs::Run::new(pty));
        view
    }

    #[test]
    fn chrome_stays_on_the_gpui_02_component_line() {
        // The sessions chrome uses gpui-component 0.5.x, the last line built
        // on gpui 0.2.2. 0.6+ moved to the gpui-pre 0.3.6 fork and would
        // force a framework migration: fail loudly so that move is conscious.
        // Manifest-relative so the test passes regardless of the
        // process working directory (CI, editors, `cargo test -p`).
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
            .expect("Cargo.lock readable in tests");
        let mut lines = lock.lines();
        let mut found = false;
        while let Some(line) = lines.next() {
            if line.trim() == r#"name = "gpui-component""# {
                let version = lines
                    .next()
                    .expect("version follows name")
                    .trim()
                    .to_string();
                assert!(
                    version.starts_with(r#"version = "0.5."#),
                    "gpui-component left 0.5.x: review migration, got {version}"
                );
                found = true;
                break;
            }
        }
        assert!(found, "gpui-component entry missing from Cargo.lock");
    }

    #[test]
    fn history_holds_exited_and_historic_but_not_live_or_pending() {
        let mut view = mixed_shell();
        let id_of = |view: &ShellView, id: &str| {
            view.app
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .clone()
        };
        // Historic provider entry with no live PTY: history.
        assert!(view.is_history(&id_of(&view, "hist")));
        // Live child: active, never history.
        assert!(!view.is_history(&id_of(&view, "live-working")));
        // Fresh run awaiting spawn (no PTY yet, no provider id): active.
        assert!(!view.is_history(&id_of(&view, "live-idle")));
        // The live child exits: it moves into History.
        let live_id = "live-working".to_string();
        view.runs.insert(
            live_id.clone(),
            crate::gui::runs::Run::new(
                crate::embedded::EmbeddedPty::spawn("true", &[], 80, 24).unwrap(),
            ),
        );
        for _ in 0..100 {
            view.refresh();
            if view.runs.get(&live_id).is_some_and(|run| run.exited()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(view.runs[&live_id].exited());
        assert!(view.is_history(&id_of(&view, "live-working")));
        // Resuming the historic entry (live PTY again) returns it to the
        // active groups until its child exits.
        let pty = crate::embedded::EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).unwrap();
        view.runs
            .insert("hist".to_string(), crate::gui::runs::Run::new(pty));
        assert!(!view.is_history(&id_of(&view, "hist")));
        // Partition order follows the list order: drop the dead child
        // (as a restart would) and the exited run leaves History; only
        // the historic entry remains.
        view.runs.remove(&live_id);
        assert!(!view.is_history(&id_of(&view, "live-working")));
        view.runs.remove("hist");
        assert_eq!(
            view.history_indices(),
            vec![1],
            "only the historic entry is history now"
        );
    }

    #[test]
    fn fresh_startup_with_only_historic_is_all_history() {
        // Issue #39 acceptance: historic entries land under collapsed
        // History, never mixed into the active groups.
        let view = ShellView::new_with_sessions(vec![
            sess("h1", Status::Idle, 1, Some("s-1")),
            sess("h2", Status::Working, 2, Some("s-2")),
        ]);
        assert!(!view.app.history_expanded);
        assert_eq!(view.history_indices().len(), 2);
    }

    #[test]
    fn jk_skips_collapsed_history_and_enters_when_expanded() {
        // Issue #39: j/k skip collapsed History by default, or enter it
        // when expanded.
        let mut view = mixed_shell();
        view.app.focus_nav();
        assert!(!view.app.history_expanded);
        view.app.selected = 0; // live-idle
        assert_eq!(
            view.nav_action("j", false),
            super::super::nav::NavAction::None
        );
        assert_eq!(view.app.selected, 2, "j skips the historic entry");
        assert_eq!(
            view.nav_action("j", false),
            super::super::nav::NavAction::None
        );
        assert_eq!(view.app.selected, 0, "j wraps among live runs");
        assert_eq!(
            view.nav_action("k", false),
            super::super::nav::NavAction::None
        );
        assert_eq!(view.app.selected, 2, "k skips it backwards too");
        // Expanded: j/k enter History like any other row.
        view.app.toggle_history();
        view.app.selected = 0;
        assert_eq!(
            view.nav_action("j", false),
            super::super::nav::NavAction::None
        );
        assert_eq!(view.app.selected, 1, "expanded history is reachable");
        // Paging skips collapsed History as well.
        view.app.toggle_history();
        view.app.selected = 0;
        assert_eq!(
            view.nav_action("pagedown", false),
            super::super::nav::NavAction::None
        );
        assert_ne!(view.app.selected, 1, "page skips collapsed history");
        assert!(view.selection_visible());
    }

    #[test]
    fn h_key_toggles_history() {
        // Issue #39: expanding/collapsing works by keyboard (`h`) as
        // well as by mouse (the toggle row click in `render_runs`).
        let mut view = mixed_shell();
        assert!(!view.app.history_expanded);
        assert_eq!(
            view.nav_action("h", false),
            super::super::nav::NavAction::None
        );
        assert!(view.app.history_expanded);
        assert_eq!(
            view.nav_action("h", false),
            super::super::nav::NavAction::None
        );
        assert!(!view.app.history_expanded);
    }

    #[test]
    fn thirty_sessions_stay_reachable_by_keyboard() {
        // Issue #40: with a full panel every session stays reachable by
        // keyboard paging (the wheel counterpart is structural: the list
        // owns the scroll container while header/footer stay pinned).
        let mut view = test_shell();
        for _ in 0..30 {
            view.app.start_new_session();
            let _ = view.app.take_pending_spawn();
        }
        view.app.focus_nav();
        view.app.selected = 0;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            seen.insert(view.app.selected_session().unwrap().id.clone());
            view.nav_action("j", false);
        }
        assert_eq!(seen.len(), 30, "every session reachable by j");
        // Paging still pages: from the head one PgDn moves PAGE_STEP.
        view.app.selected = 0;
        view.nav_action("pagedown", false);
        assert_eq!(view.app.selected, crate::app::PAGE_STEP);
        // ... and clamps at the tail.
        for _ in 0..10 {
            view.nav_action("pagedown", false);
        }
        assert_eq!(view.app.selected, 29);
        view.nav_action("pageup", false);
        assert_eq!(view.app.selected, 29 - crate::app::PAGE_STEP);
    }
}
