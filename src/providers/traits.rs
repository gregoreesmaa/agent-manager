//! Provider trait: something that can discover agent chat sessions.

use crate::app::ChatSession;

/// A source of chat sessions (Muse CLI store, mock, future providers).
/// Discovery never fails: unreachable stores yield an empty list so the
/// app still starts (degraded mode).
pub trait Provider {
    fn discover_sessions(&self) -> Vec<ChatSession>;
}
