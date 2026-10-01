//! Session-provider abstraction.

pub mod muse_cli;
pub mod opencode_cli;
pub mod traits;

pub use muse_cli::MuseCliProvider;
pub use opencode_cli::OpencodeCliProvider;
pub use traits::Provider;
