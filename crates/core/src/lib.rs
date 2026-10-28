//! `tomet-cms` core library.
//!
//! UI-less, I/O-free content management system engine for the Tomet ecosystem.

pub mod conflict;
pub mod host;
pub mod memory;
pub mod save;

pub use conflict::{
    Conflict, ConflictError, ConflictKind, ConflictSide, list_conflicts, resolve_conflict,
};
pub use host::{CommitResult, FileContent, Host, HostError, Revision};
pub use memory::MemoryHost;
pub use save::{
    DEFAULT_MAX_RETRIES, SaveError, SaveOptions, SaveResult, SaveWarning, save, save_with_options,
};
pub use tomet;
