//! Host abstraction for Git storage providers.
//!
//! Defines the minimal asynchronous interface required by `tomet-cms` to read
//! and commit documents on top of a remote Git host without a local working copy.

use std::fmt;

/// Opaque revision identifier (e.g. commit SHA or branch ref).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Revision(pub String);

impl Revision {
    /// Creates a new revision identifier.
    pub fn new(rev: impl Into<String>) -> Self {
        Self(rev.into())
    }

    /// Returns the revision identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for Revision {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Revision {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::ops::Deref for Revision {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Content of a file read from a host at a specific revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContent {
    /// Raw file content.
    pub content: String,
    /// Exact revision at which this content was resolved.
    pub revision: Revision,
}

impl FileContent {
    /// Creates a new `FileContent`.
    pub fn new(content: impl Into<String>, revision: impl Into<Revision>) -> Self {
        Self {
            content: content.into(),
            revision: revision.into(),
        }
    }
}

/// Result of committing a change to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    /// The revision produced by the commit.
    pub revision: Revision,
}

impl CommitResult {
    /// Creates a new `CommitResult`.
    pub fn new(revision: impl Into<Revision>) -> Self {
        Self {
            revision: revision.into(),
        }
    }
}

/// Errors returned by a [`Host`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// Authentication failed or token was rejected (HTTP 401).
    Unauthorized,
    /// Permission denied for the requested operation or repository (HTTP 403).
    Forbidden(String),
    /// API rate limit exceeded (HTTP 403 / 429).
    RateLimited,
    /// The requested file blob is binary or non-text and cannot be processed as a `.tmt` document.
    NonTextBlob {
        path: String,
    },
    /// The target branch moved between reading head and committing.
    BranchMoved {
        expected: Revision,
        actual: Revision,
    },
    /// The specified revision does not exist.
    RevisionNotFound(Revision),
    /// An underlying host or communication error occurred.
    Other(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => {
                write!(f, "unauthorized: authentication failed or token rejected")
            }
            Self::Forbidden(msg) => write!(f, "forbidden: {msg}"),
            Self::RateLimited => write!(f, "API rate limit exceeded"),
            Self::NonTextBlob { path } => {
                write!(f, "file blob is binary or non-text: '{path}'")
            }
            Self::BranchMoved { expected, actual } => {
                write!(
                    f,
                    "branch moved: expected head {expected}, but actual head is {actual}"
                )
            }
            Self::RevisionNotFound(rev) => write!(f, "revision not found: {rev}"),
            Self::Other(msg) => write!(f, "host error: {msg}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Abstract Git host interface.
///
/// # Concurrency & Single-Threading Contract
/// The futures returned by [`Host`] methods are deliberately **not** bounded by `Send`.
/// `tomet-cms` is designed to run in browser WebAssembly (`wasm32-unknown-unknown`),
/// which is single-threaded. Callers are responsible for driving the returned futures
/// on their local task executor.
#[allow(async_fn_in_trait)]
pub trait Host {
    /// Fetches the current head revision of the target branch.
    async fn get_head(&self) -> Result<Revision, HostError>;

    /// Reads a file at a specific revision.
    ///
    /// Returns `Ok(None)` if the file does not exist at `rev`.
    async fn read_file(
        &self,
        path: &str,
        rev: &Revision,
    ) -> Result<Option<FileContent>, HostError>;

    /// Commits a single file change on top of `expected_head`.
    ///
    /// The caller must supply a human-readable commit `message`.
    ///
    /// Must fail with [`HostError::BranchMoved`] if the current remote head
    /// no longer matches `expected_head`.
    async fn commit_file(
        &self,
        path: &str,
        content: &str,
        expected_head: &Revision,
        message: &str,
    ) -> Result<CommitResult, HostError>;
}
