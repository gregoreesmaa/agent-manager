//! agent-manager core: framework-free shared logic for native shells.
//!
//! This library is the cross-platform contract behind epic #60: the eight
//! core modules stay UI-toolkit agnostic (no toolkit, no shell code) so
//! each native shell (macOS SwiftUI first, then Linux GTK4/VTE, Windows
//! WinUI/ConPTY) binds them in-process instead of reimplementing them.
//! The existing binary (`src/main.rs` + `src/gui/`) is one consumer
//! of this crate, like any future shell. The headless test suite attached
//! to these modules is the cross-platform contract: it must pass unchanged
//! on every OS gate.

pub mod app;
pub mod config;
pub mod embedded;
pub mod ffi;
pub mod parsers;
pub mod persist;
pub mod providers;
pub mod scrollback;
pub mod transcript;
