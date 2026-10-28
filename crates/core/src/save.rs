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
    /// `true` if the committed document contains at least one `@conflict` marker.
    pub has_conflicts: bool,
    /// Any warnings produced during the save (e.g. modify/delete conflict).
    pub warnings: Vec<SaveWarning>,
}

/// Errors that can occur during a save operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Attempted to save a file that is not a `.tmt` document.
    NonTmtPath(String),
    /// Attempted to create a file that already exists at head revision.
    AlreadyExists {
        path: String,
    },
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
            Self::AlreadyExists { path } => write!(f, "file already exists: '{path}'"),
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

/// A request to save a `.tmt` document to a [`Host`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveRequest<'a> {
    /// Relative repository path of the document (must end with `.tmt`).
    pub path: &'a str,
    /// Base revision the edit was started from, or `None` if creating a new document.
    pub base_rev: Option<&'a Revision>,
    /// New content to save.
    pub local_content: &'a str,
    /// Commit message describing the change.
    pub message: &'a str,
}

impl<'a> SaveRequest<'a> {
    /// Creates a request to create a new document.
    ///
    /// If the file already exists remotely at HEAD, the save will fail with
    /// [`SaveError::AlreadyExists`] without merging.
    pub fn create(path: &'a str, content: &'a str, message: &'a str) -> Self {
        Self {
            path,
            base_rev: None,
            local_content: content,
            message,
        }
    }

    /// Creates a request to edit an existing document from a known `base_rev`.
    ///
    /// If remote changes occurred concurrently, a three-way merge will be attempted.
    pub fn edit(
        path: &'a str,
        base_rev: &'a Revision,
        content: &'a str,
        message: &'a str,
    ) -> Self {
        Self {
            path,
            base_rev: Some(base_rev),
            local_content: content,
            message,
        }
    }
}

/// Saves a document to a [`Host`], merging local changes with remote updates if necessary.
///
/// Steps:
/// 1. Validates that `request.path` ends with `.tmt`.
/// 2. Fetches current HEAD revision from `host`.
/// 3. Reads remote content at HEAD and base content at `request.base_rev`.
/// 4. If creating (`base_rev: None`) and file exists at HEAD, fails with [`SaveError::AlreadyExists`].
/// 5. Merges ASTs using `tomet::merge::merge` (or fast-path if remote == base).
/// 6. Computes `has_conflicts` from the committed text via the `conflict` module.
/// 7. Commits the result on top of HEAD with `request.message`.
/// 8. Retries automatically on [`HostError::BranchMoved`] up to the retry limit.
pub async fn save<H: Host>(
    host: &H,
    request: SaveRequest<'_>,
) -> Result<SaveResult, SaveError> {
    save_with_options(host, request, SaveOptions::default()).await
}

/// Saves a document with custom [`SaveOptions`].
pub async fn save_with_options<H: Host>(
    host: &H,
    request: SaveRequest<'_>,
    options: SaveOptions,
) -> Result<SaveResult, SaveError> {
    if !request.path.ends_with(".tmt") {
        return Err(SaveError::NonTmtPath(request.path.to_string()));
    }

    let mut attempts = 0;
    let max_retries = options.max_retries.max(1);

    while attempts < max_retries {
        attempts += 1;

        // Fetch current remote head
        let head_rev = host.get_head().await?;

        // Read remote content at current head
        let remote_file = host.read_file(request.path, &head_rev).await?;
        let remote_content = remote_file.map(|f| f.content);

        // Determine content to commit and warnings
        let (content_to_commit, warnings) = match request.base_rev {
            None => {
                // Create mode: reject immediately if file already exists at head; no merge.
                if remote_content.is_some() {
                    return Err(SaveError::AlreadyExists {
                        path: request.path.to_string(),
                    });
                }
                (request.local_content.to_string(), Vec::new())
            }
            Some(base_revision) => {
                let base_content = host.read_file(request.path, base_revision).await?.map(|f| f.content);
                match (base_content, remote_content) {
                    // File was absent at base, but remote already added it
                    (None, Some(_)) => {
                        return Err(SaveError::AlreadyExists {
                            path: request.path.to_string(),
                        });
                    }
                    // Fresh file at both base and remote
                    (None, None) => (request.local_content.to_string(), Vec::new()),
                    // Remote deleted the file while local edited it
                    (Some(_base_text), None) => {
                        let warnings = vec![SaveWarning::ModifyDeleteConflict {
                            path: request.path.to_string(),
                        }];
                        (request.local_content.to_string(), warnings)
                    }
                    // Both base and remote exist
                    (Some(base_text), Some(remote_text)) => {
                        if remote_text == base_text || remote_text == request.local_content {
                            (request.local_content.to_string(), Vec::new())
                        } else {
                            let base_doc = tomet::parser::parse_document(&base_text)
                                .map_err(|e| SaveError::ParseError(e.to_string()))?;
                            let local_doc = tomet::parser::parse_document(request.local_content)
                                .map_err(|e| SaveError::ParseError(e.to_string()))?;
                            let remote_doc = tomet::parser::parse_document(&remote_text)
                                .map_err(|e| SaveError::ParseError(e.to_string()))?;

                            let merge_res = tomet::merge::merge(&base_doc, &local_doc, &remote_doc);
                            let text = tomet::printer::document_to_tm(&merge_res.document);
                            (text, Vec::new())
                        }
                    }
                }
            }
        };

        // Determine whether committed text contains any unresolved conflict markers
        let has_conflicts = crate::conflict::list_conflicts(&content_to_commit)
            .map(|c| !c.is_empty())
            .unwrap_or_else(|_| content_to_commit.contains("@conflict"));

        // Attempt commit on top of head_rev
        match host
            .commit_file(request.path, &content_to_commit, &head_rev, request.message)
            .await
        {
            Ok(commit) => {
                return Ok(SaveResult {
                    commit,
                    has_conflicts,
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
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Edit note");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert!(res.warnings.is_empty());
        assert_eq!(
            host.get_current_file("note.tmt"),
            Some(local_text.to_string())
        );
        assert_eq!(host.last_commit_message(), Some("Edit note".to_string()));
    }

    #[test]
    fn fast_path_with_preexisting_conflict_reports_has_conflicts() {
        let conflict_text = concat!(
            "Alpha.\n\n",
            "@conflict(a: [Mine], b: [Theirs])\n\n",
            "Beta.\n"
        );
        let host = MemoryHost::with_files([("note.tmt", conflict_text)]);
        let base_rev = host.current_head();

        // Remote equals base (fast path), but content has unresolved conflict
        let req = SaveRequest::edit("note.tmt", &base_rev, conflict_text, "Keep conflict");
        let res = block_on(save(&host, req)).unwrap();

        assert!(res.has_conflicts, "should detect preexisting @conflict on fast path");
        assert!(res.warnings.is_empty());
        assert_eq!(
            host.last_commit_message(),
            Some("Keep conflict".to_string())
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
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Merge Alpha");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert!(res.warnings.is_empty());

        let final_text = host.get_current_file("note.tmt").unwrap();
        assert!(final_text.contains("Alpha changed."));
        assert!(final_text.contains("Beta changed."));
        assert!(!final_text.contains("@conflict"));
        assert_eq!(host.last_commit_message(), Some("Merge Alpha".to_string()));
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
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Conflict commit");
        let res = block_on(save(&host, req)).unwrap();

        assert!(res.has_conflicts, "expected conflict");
        assert!(res.warnings.is_empty());

        let final_text = host.get_current_file("note.tmt").unwrap();
        assert!(final_text.contains("@conflict"));
        assert!(final_text.contains("Mine"));
        assert!(final_text.contains("Theirs"));
        assert_eq!(
            host.last_commit_message(),
            Some("Conflict commit".to_string())
        );
    }

    #[test]
    fn retries_and_succeeds_when_head_moves_during_commit() {
        let base_text = "Alpha.\n";
        let host = MemoryHost::with_files([("note.tmt", base_text)]);
        let base_rev = host.current_head();

        // Simulate head moving once during commit
        host.simulate_branch_moved_times(1);

        let local_text = "Alpha updated.\n";
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Retry commit");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert_eq!(
            host.get_current_file("note.tmt"),
            Some(local_text.to_string())
        );
        assert_eq!(
            host.last_commit_message(),
            Some("Retry commit".to_string())
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
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Exceeded retries");
        let res = block_on(save(&host, req));

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
        let req = SaveRequest::edit("note.tmt", &base_rev, local_text, "Preserve deleted");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert_eq!(res.warnings.len(), 1);
        assert!(matches!(
            res.warnings[0],
            SaveWarning::ModifyDeleteConflict { .. }
        ));

        // Preserved local content without @conflict marker
        let final_text = host.get_current_file("note.tmt").unwrap();
        assert_eq!(final_text, local_text);
        assert!(!final_text.contains("@conflict"));
        assert_eq!(
            host.last_commit_message(),
            Some("Preserve deleted".to_string())
        );
    }

    #[test]
    fn create_succeeds_when_absent() {
        let host = MemoryHost::new();

        let local_text = "Fresh note.\n";
        let req = SaveRequest::create("new_note.tmt", local_text, "Create fresh note");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert!(res.warnings.is_empty());
        assert_eq!(
            host.get_current_file("new_note.tmt"),
            Some(local_text.to_string())
        );
        assert_eq!(
            host.last_commit_message(),
            Some("Create fresh note".to_string())
        );
    }

    #[test]
    fn create_fails_with_already_exists_when_present_at_head() {
        let host = MemoryHost::with_files([("note.tmt", "Existing note.\n")]);

        let req = SaveRequest::create("note.tmt", "New content.\n", "Create existing");
        let res = block_on(save(&host, req));
        match res {
            Err(SaveError::AlreadyExists { path }) => {
                assert_eq!(path, "note.tmt");
            }
            other => panic!("expected AlreadyExists, got {other:?}"),
        }

        // Host content untouched
        assert_eq!(
            host.get_current_file("note.tmt"),
            Some("Existing note.\n".to_string())
        );
    }

    #[test]
    fn create_retries_correctly_when_head_moves_but_file_still_absent() {
        let host = MemoryHost::new();
        host.simulate_branch_moved_times(1);

        let local_text = "Created after retry.\n";
        let req = SaveRequest::create("new_note.tmt", local_text, "Create after retry");
        let res = block_on(save(&host, req)).unwrap();

        assert!(!res.has_conflicts);
        assert_eq!(
            host.get_current_file("new_note.tmt"),
            Some(local_text.to_string())
        );
        assert_eq!(
            host.last_commit_message(),
            Some("Create after retry".to_string())
        );
    }

    #[test]
    fn create_loses_race_and_ends_in_already_exists() {
        let host = MemoryHost::new();
        host.simulate_concurrent_file_on_next_commit(
            "new_note.tmt",
            Some("Concurrent creator won.\n"),
        );

        let req = SaveRequest::create("new_note.tmt", "Local lost race.\n", "Create losing race");
        let res = block_on(save(&host, req));
        match res {
            Err(SaveError::AlreadyExists { path }) => {
                assert_eq!(path, "new_note.tmt");
            }
            other => panic!("expected AlreadyExists, got {other:?}"),
        }

        // Host content remains what concurrent creator wrote; never merged
        assert_eq!(
            host.get_current_file("new_note.tmt"),
            Some("Concurrent creator won.\n".to_string())
        );
    }

    #[test]
    fn non_tmt_path_returns_typed_error() {
        let host = MemoryHost::new();

        let req = SaveRequest::create("readme.md", "# Markdown", "Bad path");
        let res = block_on(save(&host, req));
        assert!(matches!(res, Err(SaveError::NonTmtPath(p)) if p == "readme.md"));
    }
}
