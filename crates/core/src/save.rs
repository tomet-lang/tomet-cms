//! Save flow and AST-level three-way merge orchestration.
//!
//! Provides the primary [`save`] entry point to fetch, merge, and commit
//! `.tmt` documents onto a remote Git host with bounded retries for optimistic concurrency.

use std::fmt;

use crate::host::{CommitResult, Host, HostError, Revision};

/// Default maximum number of commit attempts when branch movements occur.
pub const DEFAULT_MAX_RETRIES: usize = 5;

/// Options configuring the save flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveOptions {
    /// Maximum number of commit attempts when encountering [`HostError::BranchMoved`].
    pub max_retries: usize,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }
}

/// Warnings produced during the save operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveWarning {
    /// The remote file was deleted while local edits were being made.
    ///
    /// Following Git's modify/delete conflict policy, the local edited content
    /// is preserved and committed.
    ///
    /// Note: This behaviour is provisional.
    ModifyDeleteConflict {
        path: String,
    },
}

impl fmt::Display for SaveWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModifyDeleteConflict { path } => {
                write!(
                    f,
                    "modify/delete conflict: remote deleted '{path}' while local edited it; kept local content (provisional)"
                )
            }
        }
    }
}

/// The result of a successful save operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveResult {
    /// Commit produced on the host.
    pub commit: CommitResult,
    /// `true` if the save merged cleanly without unresolved `@conflict` markers.
    pub clean: bool,
    /// Any warnings produced during the save (e.g. modify/delete conflict).
    pub warnings: Vec<SaveWarning>,
}

/// Errors that can occur during a save operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Attempted to save a file that is not a `.tmt` document.
    NonTmtPath(String),
    /// Head branch kept moving and exceeded the retry limit.
    MaxRetriesExceeded {
        attempts: usize,
    },
    /// Failed to parse a document AST during three-way merge.
    ParseError(String),
    /// An underlying host error occurred.
    Host(HostError),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonTmtPath(p) => write!(f, "only .tmt files are supported, got '{p}'"),
            Self::MaxRetriesExceeded { attempts } => {
                write!(f, "branch kept moving; exceeded limit of {attempts} attempts")
            }
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
            Self::Host(err) => write!(f, "host error: {err}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<HostError> for SaveError {
    fn from(err: HostError) -> Self {
        Self::Host(err)
    }
}

/// Saves a document to a [`Host`], merging local changes with remote updates if necessary.
///
/// Steps:
/// 1. Validates that `path` ends with `.tmt`.
/// 2. Fetches current HEAD revision from `host`.
/// 3. Reads remote content at HEAD and base content at `base_rev`.
/// 4. Merges ASTs using `tomet::merge::merge` (or fast-path if remote == base).
/// 5. Commits the result on top of HEAD.
/// 6. Retries automatically on [`HostError::BranchMoved`] up to the retry limit.
pub async fn save<H: Host>(
    host: &H,
    path: &str,
    base_rev: Option<&Revision>,
    local_content: &str,
) -> Result<SaveResult, SaveError> {
    save_with_options(host, path, base_rev, local_content, SaveOptions::default()).await
}

/// Saves a document with custom [`SaveOptions`].
pub async fn save_with_options<H: Host>(
    host: &H,
    path: &str,
    base_rev: Option<&Revision>,
    local_content: &str,
    options: SaveOptions,
) -> Result<SaveResult, SaveError> {
    if !path.ends_with(".tmt") {
        return Err(SaveError::NonTmtPath(path.to_string()));
    }

    let mut attempts = 0;
    let max_retries = options.max_retries.max(1);

    while attempts < max_retries {
        attempts += 1;

        // Fetch current remote head
        let head_rev = host.get_head().await?;

        // Read remote content at current head
        let remote_file = host.read_file(path, &head_rev).await?;
        let remote_content = remote_file.map(|f| f.content);

        // Read base content at base revision (if base_rev provided)
        let base_content = match base_rev {
            Some(rev) => host.read_file(path, rev).await?.map(|f| f.content),
            None => None,
        };

        // Determine content to commit, clean status, and warnings
        let (content_to_commit, clean, warnings) = match (base_content, remote_content) {
            // Fresh file: no base and no remote
            (None, None) => (local_content.to_string(), true, Vec::new()),

            // Fresh file locally, but remote already added a file
            (None, Some(remote_text)) => {
                let base_doc = tomet::ast::Document::default();
                let local_doc = tomet::parser::parse_document(local_content)
                    .map_err(|e| SaveError::ParseError(e.to_string()))?;
                let remote_doc = tomet::parser::parse_document(&remote_text)
                    .map_err(|e| SaveError::ParseError(e.to_string()))?;

                let merge_res = tomet::merge::merge(&base_doc, &local_doc, &remote_doc);
                let text = tomet::printer::document_to_tm(&merge_res.document);
                (text, merge_res.clean, Vec::new())
            }

            // Remote deleted the file while local edited it
            (Some(_base_text), None) => {
                // Keep local edited content, record typed provisional warning
                let warnings = vec![SaveWarning::ModifyDeleteConflict {
                    path: path.to_string(),
                }];
                (local_content.to_string(), false, warnings)
            }

            // Both base and remote exist
            (Some(base_text), Some(remote_text)) => {
                // Fast path: remote hasn't changed since base
                if remote_text == base_text {
                    (local_content.to_string(), true, Vec::new())
                } else if remote_text == local_content {
                    // Both changed to identical content
                    (local_content.to_string(), true, Vec::new())
                } else {
                    // Three-way AST merge
                    let base_doc = tomet::parser::parse_document(&base_text)
                        .map_err(|e| SaveError::ParseError(e.to_string()))?;
                    let local_doc = tomet::parser::parse_document(local_content)
                        .map_err(|e| SaveError::ParseError(e.to_string()))?;
                    let remote_doc = tomet::parser::parse_document(&remote_text)
                        .map_err(|e| SaveError::ParseError(e.to_string()))?;

                    let merge_res = tomet::merge::merge(&base_doc, &local_doc, &remote_doc);
                    let text = tomet::printer::document_to_tm(&merge_res.document);
                    (text, merge_res.clean, Vec::new())
                }
            }
        };

        // Attempt commit on top of head_rev
        match host.commit_file(path, &content_to_commit, &head_rev).await {
            Ok(commit) => {
                return Ok(SaveResult {
                    commit,
                    clean,
                    warnings,
                });
            }
            Err(HostError::BranchMoved { .. }) => {
                // Loop to retry with updated head
                continue;
            }
            Err(err) => return Err(SaveError::Host(err)),
        }
    }

    Err(SaveError::MaxRetriesExceeded { attempts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryHost;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    fn block_on<F: Future>(fut: F) -> F::Output {
        let mut pinned = std::pin::pin!(fut);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(res) => res,
            Poll::Pending => panic!("future unexpectedly pending"),
        }
    }

    #[test]
    fn fast_path_when_remote_equals_base() {
        let initial_text = "Alpha.\n\nBeta.\n";
        let host = MemoryHost::with_files([("note.tmt", initial_text)]);
        let base_rev = host.current_head();

        let local_text = "Alpha changed.\n\nBeta.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text)).unwrap();

        assert!(res.clean);
        assert!(res.warnings.is_empty());
        assert_eq!(
            host.get_current_file("note.tmt"),
            Some(local_text.to_string())
        );
    }

    #[test]
    fn clean_merge_when_different_nodes_changed() {
        let base_text = "Alpha.\n\nBeta.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Other party changed Beta
        host.simulate_remote_commit("note.tmt", Some("Alpha.\n\nBeta changed.\n"));

        // Local changed Alpha
        let local_text = "Alpha changed.\n\nBeta.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text)).unwrap();

        assert!(res.clean);
        assert!(res.warnings.is_empty());

        let final_text = host.get_current_file("note.tmt").unwrap();
        assert!(final_text.contains("Alpha changed."));
        assert!(final_text.contains("Beta changed."));
        assert!(!final_text.contains("@conflict"));
    }

    #[test]
    fn conflict_merge_when_same_node_changed_still_committed() {
        let base_text = "Alpha.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Other party changed Alpha
        host.simulate_remote_commit("note.tmt", Some("Theirs.\n"));

        // Local also changed Alpha
        let local_text = "Mine.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text)).unwrap();

        assert!(!res.clean, "expected conflict");
        assert!(res.warnings.is_empty());

        let final_text = host.get_current_file("note.tmt").unwrap();
        assert!(final_text.contains("@conflict"));
        assert!(final_text.contains("Mine"));
        assert!(final_text.contains("Theirs"));
    }

    #[test]
    fn retries_and_succeeds_when_head_moves_during_commit() {
        let base_text = "Alpha.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Simulate head moving once during commit
        host.simulate_branch_moved_times(1);

        let local_text = "Alpha updated.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text)).unwrap();

        assert!(res.clean);
        assert_eq!(
            host.get_current_file("note.tmt"),
            Some(local_text.to_string())
        );
    }

    #[test]
    fn fails_with_max_retries_exceeded_when_head_keeps_moving() {
        let base_text = "Alpha.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Simulate head moving 10 times (more than default limit 5)
        host.simulate_branch_moved_times(10);

        let local_text = "Alpha updated.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text));

        match res {
            Err(SaveError::MaxRetriesExceeded { attempts }) => {
                assert_eq!(attempts, DEFAULT_MAX_RETRIES);
            }
            other => panic!("expected MaxRetriesExceeded, got {other:?}"),
        }
    }

    #[test]
    fn modify_delete_conflict_preserves_content_with_warning() {
        let base_text = "Alpha.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Other party deleted the file
        host.simulate_remote_commit("note.tmt", None);

        // Local edited it
        let local_text = "Alpha local edited.\n";
        let res = block_on(save(&host, "note.tmt", Some(&base_rev), local_text)).unwrap();

        assert!(!res.clean);
        assert_eq!(res.warnings.len(), 1);
        assert!(matches!(
            res.warnings[0],
            SaveWarning::ModifyDeleteConflict { .. }
        ));

        // Preserved local content without @conflict marker
        let final_text = host.get_current_file("note.tmt").unwrap();
        assert_eq!(final_text, local_text);
        assert!(!final_text.contains("@conflict"));
    }

    #[test]
    fn file_created_fresh_no_base() {
        let host = MemoryHost::new();

        let local_text = "Fresh note.\n";
        let res = block_on(save(&host, "new_note.tmt", None, local_text)).unwrap();

        assert!(res.clean);
        assert!(res.warnings.is_empty());
        assert_eq!(
            host.get_current_file("new_note.tmt"),
            Some(local_text.to_string())
        );
    }

    #[test]
    fn non_tmt_path_returns_typed_error() {
        let host = MemoryHost::new();

        let res = block_on(save(&host, "readme.md", None, "# Markdown"));
        assert!(matches!(res, Err(SaveError::NonTmtPath(p)) if p == "readme.md"));
    }
}
