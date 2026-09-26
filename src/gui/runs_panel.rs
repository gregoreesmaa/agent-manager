//! Sessions-panel render for [`super::shell::ShellView`].
//!
//! Library chrome: a `Sidebar` with one group per status section (Needs
//! input / Idle / Active), session rows, and every accumulated parsed
//! link as a click-to-copy child row — display-capped with an `N more`
//! disclosure over fully retained data. Keyboard link focus (`o`/Enter)
//! highlights the focused row via `active`.

use gpui::{
    div, px, rgb, AnyElement, ClipboardItem, Context, ElementId, IntoElement, ParentElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    Sizable as _,
};

use crate::app::{section_title, status_sections};

use super::layout::LEFT_WIDTH;
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
        if self.app.sessions.is_empty() {
            footer = footer.child(
                div()
                    .text_color(rgb(super::theme::SECONDARY_FG))
                    .text_xs()
                    .child("No sessions yet.".to_string()),
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
        let mut sidebar = Sidebar::left().w(px(LEFT_WIDTH)).header(
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
        );
        if self.app.sessions.is_empty() {
            return sidebar
                .footer(self.sidebar_footer(viewport_w))
                .into_any_element();
        }
        for (status, indices) in status_sections(&self.app.sessions) {
            let marker = super::theme::group_marker(status);
            let items: Vec<SidebarMenuItem> = indices
                .into_iter()
                .map(|i| {
                    let s = &self.app.sessions[i];
                    // Non-blank idle marker: rows stay distinguishable
                    // with color removed (see theme::row_marker).
                    let row_marker = super::theme::row_marker(s.status);
                    let row_id = s.id.clone();
                    // Parsed links as child items: click copies the full
                    // URL. Display-capped: the first MAX_VISIBLE_LINKS
                    // rows stay live and the rest fold behind an N more
                    // disclosure; the full lists stay retained (and
                    // copyable via yank) in the session. `o`/Enter does
                    // the same from the keyboard when a PR row holds link
                    // focus (highlighted via `active`).
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
                        links.push(SidebarMenuItem::new(short).active(focused).on_click(
                            cx.listener(move |this, _ev, _window, cx| {
                                this.link_cursor = Some(li);
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                this.app.set_status("copied PR link");
                            }),
                        ));
                    }
                    let (shown_rel, hidden_rel) = crate::app::visible_links(&s.related_links);
                    for link in shown_rel {
                        let short = short_of(link);
                        let url = link.clone();
                        links.push(SidebarMenuItem::new(short).on_click(cx.listener(
                            move |this, _ev, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                this.app.set_status("copied link");
                            },
                        )));
                    }
                    let hidden = hidden_prs + hidden_rel;
                    if hidden > 0 {
                        links.push(SidebarMenuItem::new(format!("… {hidden} more")));
                    }
                    if s.links_truncated {
                        links.push(SidebarMenuItem::new("(capped at 50 per list)".to_string()));
                    }
                    SidebarMenuItem::new(format!("{row_marker} {}", s.title))
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
