//! Ratatui rendering: left session list + right project detail panes.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;

/// Draw the two-pane layout.
pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(frame.area());

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let marker = match s.status {
                crate::app::Status::Attention => "!",
                crate::app::Status::Idle => " ",
                crate::app::Status::Working => ">",
            };
            let style = if i == app.selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut lines = vec![Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(format!("[{}] {}", s.status_label(), s.title), style),
            ])];
            for link in &s.pr_links {
                lines.push(Line::from(vec![Span::styled(
                    format!("    {link}"),
                    Style::default().fg(Color::Blue),
                )]));
            }
            ListItem::new(lines)
        })
        .collect();

    let mut state = ListState::default();
    if !app.sessions.is_empty() {
        state.select(Some(app.selected));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Chats"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let detail = match app.selected_session() {
        Some(s) => {
            let mut lines = vec![
                Line::from(format!("project: {}", s.project)),
                Line::from(format!("status:  {}", s.status_label())),
                Line::from(format!("id:      {}", s.id)),
                Line::from(""),
                Line::from("PR links:"),
            ];
            if s.pr_links.is_empty() {
                lines.push(Line::from("  (none)"));
            } else {
                for link in &s.pr_links {
                    lines.push(Line::from(format!("  - {link}")));
                }
            }
            Paragraph::new(lines)
        }
        None => Paragraph::new("No sessions. (q to quit)"),
    }
    .block(Block::default().borders(Borders::ALL).title("Project"));
    frame.render_widget(detail, chunks[1]);
}
