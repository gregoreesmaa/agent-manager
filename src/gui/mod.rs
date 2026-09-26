//! Native gpui shell (runs panel, terminal pane, status bar).
//!
//! Layout of the volatility shield: [`keys`] and [`terminal`] are
//! framework-free and unit-tested; [`shell`] is the thin gpui view over
//! focused modules — [`runs`] (the single live-run struct), [`spawn`]
//! (spawn/key paths), [`lifecycle`] (restart/close/quit), [`pump`]
//! (dirty-gated pump), [`nav`] (key dispatch), [`runs_panel`] (sessions
//! panel), [`terminal_pane`] (terminal render, selection, clipboard),
//! [`pager`] (scrollback pager over the retained output buffer).

pub mod attention;
pub mod comfort;
pub mod keys;
pub mod layout;
pub mod lifecycle;
pub mod nav;
pub mod pager;
pub mod pump;
pub mod runs;
pub mod runs_panel;
pub mod shell;
pub mod spawn;
pub mod terminal;
pub mod terminal_pane;
pub mod theme;
pub mod view;
