//! Session-provider abstraction.

pub mod antigravity_cli;
pub mod claude_cli;
pub mod codex_cli;
pub mod muse_cli;
pub mod opencode_cli;
pub mod traits;

pub use antigravity_cli::AntigravityCliProvider;
pub use claude_cli::ClaudeCliProvider;
pub use codex_cli::CodexCliProvider;
pub use muse_cli::MuseCliProvider;
pub use opencode_cli::OpencodeCliProvider;
pub use traits::Provider;
