//! Session-provider abstraction.

pub mod muse_cli;
pub mod traits;

pub use muse_cli::MuseCliProvider;
pub use traits::{Provider, ProviderError};
