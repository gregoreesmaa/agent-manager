//! agent-manager: native Rust CLI harness listing agent chat sessions.
//!
//! Left pane: top-down chat list (attention > idle > working).
//! Right pane: selected project detail incl. per-chat GitHub PR links.
//! Keys: j/k or Up/Down to move, q/Esc to quit.

mod app;
mod parsers;
mod providers;
mod ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::parsers::registry::RegistryParser;
use crate::providers::{MuseCliProvider, Provider};

fn main() -> Result<()> {
    let provider = MuseCliProvider::new(
        MuseCliProvider::default_store_root(),
        Box::new(RegistryParser::default()),
    );
    let sessions = provider.discover_sessions().unwrap_or_default();
    let mut app = App::new(sessions);

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;
        if !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                _ => {}
            }
        }
    }
    Ok(())
}
