//! Native gpui shell (runs panel, terminal pane, status bar).
//!
//! Layout of the volatility shield: [`keys`] and [`terminal`] are
//! framework-free and unit-tested; [`shell`] is the thin gpui view.

pub mod keys;
pub mod shell;
pub mod terminal;
