//! Provider trait: something that can discover agent chat sessions.

use crate::app::ChatSession;

/// A source of chat sessions (Muse CLI store, mock, future providers).
pub trait Provider {
    /// Short identifier for startup diagnostics (e.g. `"muse-cli"`).
    fn name(&self) -> &'static str;
    fn discover_sessions(&self) -> Result<Vec<ChatSession>, ProviderError>;
}

/// Provider failure modes.
///
/// No variant is constructed today by design: discovery degrades to an
/// empty list on unreachable stores (see `MuseCliProvider` docs), and
/// startup funnels `Err` into that same empty list. The type stays so a
/// future provider can fail loudly without changing the seam.
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
