//! Keystroke → PTY bytes for the gpui shell.
//!
//! Re-export of the framework-free core table (`crate::keys`): the gpui
//! shell is one consumer among four, not the owner. The mapping and its
//! pinning tests live in the core; this module only keeps the import path
//! (`super::keys::…`) compiling.

pub use crate::keys::{keystroke_to_pty, KeyPress};
