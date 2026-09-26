//! Provider trait: something that can discover agent chat sessions.

use crate::app::ChatSession;

/// A source of chat sessions (Muse CLI store, mock, future providers).
pub trait Provider {
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    fn discover_sessions(&self) -> Result<Vec<ChatSession>, ProviderError>;
}

/// Provider failure modes.
#[allow(dead_code)]
#[derive(Debug)]
pub enum ProviderError {
    /// Backing store unreachable (missing dir, permission denied, ...).
    Unreachable(String),
    /// Store reachable but a record could not be understood.
    Parse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unreachable(msg) => write!(f, "provider unreachable: {msg}"),
            ProviderError::Parse(msg) => write!(f, "provider parse error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}
