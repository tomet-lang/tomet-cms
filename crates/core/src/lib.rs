//! `tomet-cms` core library.
//!
//! UI-less, I/O-free content management system engine for the Tomet ecosystem.

pub mod host;
pub mod memory;
pub mod save;

pub use host::{CommitResult, FileContent, Host, HostError, Revision};
pub use memory::MemoryHost;
pub use save::{
    DEFAULT_MAX_RETRIES, SaveError, SaveOptions, SaveRequest, SaveResult, SaveWarning, save,
    save_with_options,
};
pub use tomet;
