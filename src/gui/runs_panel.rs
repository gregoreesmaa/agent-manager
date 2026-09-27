//! Sessions-panel render for [`super::shell::ShellView`].
//!
//! Library chrome: a `Sidebar` with one group per status section (Needs
//! input / Idle / Active), session rows (each with its folder tail when
//! the session picked one, issue #48), and every accumulated parsed
//! link as a child row — display-capped with an `N more` disclosure
//! over fully retained data. Clicking a link opens it in the default
//! browser and copies it; modifier-click copies only (issue #49).
//! Keyboard link focus (`o`/Enter/`y`) highlights the focused row via
//! `active`. The header doubles as the window drag region now that the
//! OS title bar is hidden (issue #44).

use gpui::{
    div, px, rgb, AnyElement, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Styled, WindowControlArea,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    Sizable as _,
};

use crate::app::{section_title, status_sections};

use super::shell::ShellView;

impl ShellView {
    /// Sessions panel as library chrome: a [`Sidebar`] with one
    /// [`SidebarGroup`] per status section (Needs input / Idle / Active),
    /// session rows as [`SidebarMenuItem`]s (active = selected), and every
    /// accumulated parsed link as a child item that copies its URL on
    /// click. The sidebar scrolls internally, so long link lists never
    /// push sessions off-panel. The terminal pane stays hand-rolled gpui.
    /// Sidebar footer (issue #32): the status line lives here instead
    /// of a dedicated bar, giving the terminal maximum vertical space.
    /// The empty-state line rides above it while there are no sessions.
    pub(crate) fn sidebar_footer(&self, viewport_w: f32) -> gpui::Div {
        let mut footer = div().flex().flex_col();
        // Title filter state (issue #29): the panel stays filtered until
        // `/` then Esc clears it.
        if !self.app.filter.is_empty() {
            let shown = self.app.visible_indices().len();
            footer = footer.child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .text_xs()
                    .child(format!(
                        "filter '{}' · {shown} runs · / then Esc clears",
                        self.app.filter
                    )),
            );
        }
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

    pub(crate) fn render_runs(&self, cx: &mut Context<Self>, viewport_w: f32) -> AnyElement {
        // Header with a real button: sessions start here, not at a key hint.
        // The footer carries the status line (issue #32), so the app name
        // and hints live in the panel, not in their own bar.
        // Width follows the persisted comfort setting (issue #29).
        // Issue #44: with the OS title bar hidden the header is the
        // window drag region (traffic lights float over the content).
        // Child controls keep their own hitboxes, so `+ New` still
        // clicks while the bare header drags the window.
        let mut sidebar = Sidebar::left().w(px(self.app.sidebar_width())).header(
            div().window_control_area(WindowControlArea::Drag).child(
                SidebarHeader::new().child("Sessions".to_string()).child(
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
            ),
        );
        if self.app.sessions.is_empty() {
            return sidebar
                .footer(self.sidebar_footer(viewport_w))
                .into_any_element();
        }
        for (status, indices) in status_sections(&self.app.sessions) {
            let marker = super::theme::group_marker(status);
            // Title filter (issue #29): non-matching runs hide without
            // changing sort order; emptied groups drop out entirely.
            let items: Vec<SidebarMenuItem> = indices
                .into_iter()
                .filter(|i| self.app.matches_filter(&self.app.sessions[*i]))
                .map(|i| {
                    let s = &self.app.sessions[i];
                    // Non-blank idle marker: rows stay distinguishable
                    // with color removed (see theme::row_marker).
                    let row_marker = super::theme::row_marker(s.status);
                    let row_id = s.id.clone();
                    // Parsed links as child items: click opens the URL
                    // and copies it, modifier-click copies only.
                    // Display-capped: the first MAX_VISIBLE_LINKS rows
                    // stay live and the rest fold behind an N more
                    // disclosure; the full lists stay retained (and
                    // copyable via yank) in the session. `o` focuses,
                    // Enter opens, `y` copies from the keyboard when a
                    // row holds link focus (highlighted via `active`).
                    let short_of = |link: &str| {
                        link.trim_start_matches("https://")
                            .trim_start_matches("http://")
                            .trim_start_matches("github.com/")
                            .to_string()
                    };
                    let mut links: Vec<SidebarMenuItem> = Vec::new();
                    let (shown_prs, hidden_prs) = crate::app::visible_links(&s.pr_links);
                    for (li, link) in shown_prs.iter().enumerate() {
                        let short = short_of(link);
                        let url = link.clone();
                        let focused = i == self.app.selected && self.link_cursor == Some(li);
                        // Issue #49: plain click opens the exact URL in
                        // the default browser and copies it;
                        // Cmd/Ctrl/Shift-click copies without opening.
                        links.push(SidebarMenuItem::new(short).active(focused).on_click(
                            cx.listener(move |this, ev: &ClickEvent, _window, cx| {
                                this.link_cursor = Some(li);
                                let m = ev.modifiers();
                                let copy_only = m.platform || m.control || m.shift;
                                this.activate_link(&url, true, copy_only, cx);
                            }),
                        ));
                    }
                    let (shown_rel, hidden_rel) = crate::app::visible_links(&s.related_links);
                    // Related (issue/commit/file) links share the same
                    // open/copy treatment; their link-cursor indices
                    // follow the PR rows so `o`/Enter reaches them too.
                    let pr_count = s.pr_links.len();
                    for (ri, link) in shown_rel.iter().enumerate() {
                        let short = short_of(link);
                        let url = link.clone();
                        let cursor = pr_count + ri;
                        let focused = i == self.app.selected && self.link_cursor == Some(cursor);
                        links.push(SidebarMenuItem::new(short).active(focused).on_click(
                            cx.listener(move |this, ev: &ClickEvent, _window, cx| {
                                this.link_cursor = Some(cursor);
                                let m = ev.modifiers();
                                let copy_only = m.platform || m.control || m.shift;
                                this.activate_link(&url, false, copy_only, cx);
                            }),
                        ));
                    }
                    let hidden = hidden_prs + hidden_rel;
                    if hidden > 0 {
                        links.push(SidebarMenuItem::new(format!("… {hidden} more")));
                    }
                    if s.links_truncated {
                        links.push(SidebarMenuItem::new("(capped at 50 per list)".to_string()));
                    }
                    // Issue #48: the session's folder tail rides the row
                    // so multi-folder work is glanceable; the full path
                    // lives in the footer detail line.
                    let row_label = match s.cwd.as_deref() {
                        Some(dir) => {
                            format!("{row_marker} {} · {}", s.title, crate::app::short_cwd(dir))
                        }
                        None => format!("{row_marker} {}", s.title),
                    };
                    SidebarMenuItem::new(row_label)
                        .active(i == self.app.selected)
                        .default_open(true)
                        .on_click(cx.listener(move |this, _ev, window, _cx| {
                            if let Some(pos) = this.app.sessions.iter().position(|s| s.id == row_id)
                            {
                                this.app.selected = pos;
                            }
                            this.clear_selection();
                            this.link_cursor = None;
                            this.focus_list(window);
                        }))
                        .children(links)
                })
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
            sidebar = sidebar.child(group);
        }
        sidebar
            .footer(self.sidebar_footer(viewport_w))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
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
}
