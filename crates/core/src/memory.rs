//! In-memory host implementation for testing and offline simulations.
//!
//! Provides a deterministic, network-free [`Host`] implementation that can
//! simulate remote repository changes, concurrent edits, deletions, and branch movements.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::host::{CommitResult, FileContent, Host, HostError, Revision};

#[derive(Debug, Clone)]
struct MemoryHostState {
    head: Revision,
    commits: BTreeMap<Revision, BTreeMap<String, String>>,
    commit_messages: BTreeMap<Revision, String>,
    commits_log: Vec<(Revision, String)>,
    commit_counter: usize,
    branch_moved_countdown: usize,
    pending_concurrent_file: Option<(String, Option<String>)>,
}

impl MemoryHostState {
    fn new_revision(&mut self) -> Revision {
        self.commit_counter += 1;
        Revision::new(format!("rev-{}", self.commit_counter))
    }
}

/// An in-memory [`Host`] implementation.
///
/// Wraps its internal state in an [`Rc`]/[`RefCell`], so cloning a [`MemoryHost`]
/// produces a handle sharing the same state. This allows test code to simulate
/// external changes (e.g. concurrent remote commits) while a save flow holds a reference.
#[derive(Debug, Clone)]
pub struct MemoryHost {
    state: Rc<RefCell<MemoryHostState>>,
}

impl Default for MemoryHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryHost {
    /// Creates a new empty `MemoryHost` with an initial revision (`rev-0`).
    pub fn new() -> Self {
        let initial_rev = Revision::new("rev-0");
        let mut commits = BTreeMap::new();
        commits.insert(initial_rev.clone(), BTreeMap::new());
        let mut commit_messages = BTreeMap::new();
        commit_messages.insert(initial_rev.clone(), "Initial commit".to_string());
        let commits_log = vec![(initial_rev.clone(), "Initial commit".to_string())];

        Self {
            state: Rc::new(RefCell::new(MemoryHostState {
                head: initial_rev,
                commits,
                commit_messages,
                commits_log,
                commit_counter: 0,
                branch_moved_countdown: 0,
                pending_concurrent_file: None,
            })),
        }
    }

    /// Creates a new `MemoryHost` initialized with the given files at `rev-0`.
    pub fn with_files<I, P, C>(files: I) -> Self
    where
        I: IntoIterator<Item = (P, C)>,
        P: Into<String>,
        C: Into<String>,
    {
        let host = Self::new();
        let mut map = BTreeMap::new();
        for (path, content) in files {
            map.insert(path.into(), content.into());
        }
        let head = host.current_head();
        host.state.borrow_mut().commits.insert(head, map);
        host
    }

    /// Returns the current HEAD revision identifier.
    pub fn current_head(&self) -> Revision {
        self.state.borrow().head.clone()
    }

    /// Reads a file's content at the current HEAD synchronously.
    pub fn get_current_file(&self, path: &str) -> Option<String> {
        let head = self.current_head();
        self.get_file_at(&head, path)
    }

    /// Reads a file's content at a specific revision synchronously.
    pub fn get_file_at(&self, rev: &Revision, path: &str) -> Option<String> {
        self.state
            .borrow()
            .commits
            .get(rev)
            .and_then(|files| files.get(path).cloned())
    }

    /// Sets or updates a file directly at current HEAD without creating a new revision.
    pub fn set_file_at_head(&self, path: impl Into<String>, content: impl Into<String>) {
        let mut state = self.state.borrow_mut();
        let head = state.head.clone();
        if let Some(files) = state.commits.get_mut(&head) {
            files.insert(path.into(), content.into());
        }
    }

    /// Returns the commit message of the most recent commit, if any.
    pub fn last_commit_message(&self) -> Option<String> {
        self.state
            .borrow()
            .commits_log
            .last()
            .map(|(_, msg)| msg.clone())
    }

    /// Returns the commit message for a specific revision, if found.
    pub fn commit_message(&self, rev: &Revision) -> Option<String> {
        self.state
            .borrow()
            .commit_messages
            .get(rev)
            .cloned()
    }

    /// Returns the full list of committed revisions and their messages in creation order.
    pub fn commits_log(&self) -> Vec<(Revision, String)> {
        self.state.borrow().commits_log.clone()
    }

    /// Simulates another party advancing the head to an arbitrary revision with current files.
    pub fn advance_head(&self, new_rev: impl Into<Revision>) {
        let mut state = self.state.borrow_mut();
        let head = state.head.clone();
        let files = state.commits.get(&head).cloned().unwrap_or_default();
        let rev = new_rev.into();
        state.commits.insert(rev.clone(), files);
        state.commit_messages.insert(rev.clone(), "Advance head".to_string());
        state.commits_log.push((rev.clone(), "Advance head".to_string()));
        state.head = rev;
    }

    /// Simulates a remote commit by another party modifying, adding, or deleting a file.
    ///
    /// If `content` is `Some(...)`, the file is added or updated.
    /// If `content` is `None`, the file is deleted.
    /// Advances HEAD to a newly generated revision and returns it.
    pub fn simulate_remote_commit(&self, path: &str, content: Option<&str>) -> Revision {
        self.simulate_remote_commit_with_message(path, content, "Simulated remote commit")
    }

    /// Simulates a remote commit with a specific commit message.
    pub fn simulate_remote_commit_with_message(
        &self,
        path: &str,
        content: Option<&str>,
        message: &str,
    ) -> Revision {
        let mut state = self.state.borrow_mut();
        let head = state.head.clone();
        let mut files = state.commits.get(&head).cloned().unwrap_or_default();

        match content {
            Some(text) => {
                files.insert(path.to_string(), text.to_string());
            }
            None => {
                files.remove(path);
            }
        }

        let new_rev = state.new_revision();
        state.commits.insert(new_rev.clone(), files);
        state.commit_messages.insert(new_rev.clone(), message.to_string());
        state.commits_log.push((new_rev.clone(), message.to_string()));
        state.head = new_rev.clone();
        new_rev
    }

    /// Simulates branch movement when a commit is attempted.
    ///
    /// The next `times` calls to `commit_file` will automatically advance HEAD
    /// just before checking the expected head, causing a [`HostError::BranchMoved`].
    pub fn simulate_branch_moved_times(&self, times: usize) {
        self.state.borrow_mut().branch_moved_countdown = times;
    }

    /// Simulates another party committing a specific file change concurrently.
    ///
    /// On the next call to `commit_file`, HEAD is advanced with `path` modified
    /// (or deleted if `content` is `None`), causing [`HostError::BranchMoved`].
    pub fn simulate_concurrent_file_on_next_commit(
        &self,
        path: impl Into<String>,
        content: Option<impl Into<String>>,
    ) {
        self.state.borrow_mut().pending_concurrent_file =
            Some((path.into(), content.map(Into::into)));
    }
}

impl Host for MemoryHost {
    async fn get_head(&self) -> Result<Revision, HostError> {
        Ok(self.current_head())
    }

    async fn read_file(
        &self,
        path: &str,
        rev: &Revision,
    ) -> Result<Option<FileContent>, HostError> {
        let state = self.state.borrow();
        match state.commits.get(rev) {
            Some(files) => Ok(files
                .get(path)
                .map(|content| FileContent::new(content.clone(), rev.clone()))),
            None => Err(HostError::RevisionNotFound(rev.clone())),
        }
    }

    async fn commit_file(
        &self,
        path: &str,
        content: &str,
        expected_head: &Revision,
        message: &str,
    ) -> Result<CommitResult, HostError> {
        let mut state = self.state.borrow_mut();

        // If a concurrent file simulation is queued, execute it before checking head.
        if let Some((p, c)) = state.pending_concurrent_file.take() {
            let head = state.head.clone();
            let mut files = state.commits.get(&head).cloned().unwrap_or_default();
            match c {
                Some(text) => {
                    files.insert(p, text);
                }
                None => {
                    files.remove(&p);
                }
            }
            let concurrent_rev = state.new_revision();
            state.commits.insert(concurrent_rev.clone(), files);
            state.commit_messages.insert(
                concurrent_rev.clone(),
                "Simulated concurrent commit".to_string(),
            );
            state.commits_log.push((
                concurrent_rev.clone(),
                "Simulated concurrent commit".to_string(),
            ));
            state.head = concurrent_rev;
        }

        // If simulation countdown is active, advance head to simulate another party committing first.
        if state.branch_moved_countdown > 0 {
            state.branch_moved_countdown -= 1;
            let head = state.head.clone();
            let mut files = state.commits.get(&head).cloned().unwrap_or_default();
            // Touch a concurrent marker file
            files.insert(
                ".concurrent-marker".to_string(),
                format!("head moved at step {}", state.commit_counter),
            );
            let concurrent_rev = state.new_revision();
            state.commits.insert(concurrent_rev.clone(), files);
            state.commit_messages.insert(
                concurrent_rev.clone(),
                "Simulated branch moved commit".to_string(),
            );
            state.commits_log.push((
                concurrent_rev.clone(),
                "Simulated branch moved commit".to_string(),
            ));
            state.head = concurrent_rev;
        }

        if state.head != *expected_head {
            return Err(HostError::BranchMoved {
                expected: expected_head.clone(),
                actual: state.head.clone(),
            });
        }

        let head = state.head.clone();
        let mut files = state.commits.get(&head).cloned().unwrap_or_default();
        files.insert(path.to_string(), content.to_string());

        let new_rev = state.new_revision();
        state.commits.insert(new_rev.clone(), files);
        state.commit_messages.insert(new_rev.clone(), message.to_string());
        state.commits_log.push((new_rev.clone(), message.to_string()));
        state.head = new_rev.clone();

        Ok(CommitResult::new(new_rev))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn initial_state_and_read() {
        let host = MemoryHost::with_files([("doc.tmt", "Hello world.\n")]);
        let head = host.current_head();
        assert_eq!(head.as_str(), "rev-0");
        assert_eq!(
            host.get_current_file("doc.tmt"),
            Some("Hello world.\n".to_string())
        );
        assert_eq!(host.get_current_file("missing.tmt"), None);
    }

    #[test]
    fn commit_advances_head() {
        let host = MemoryHost::with_files([("doc.tmt", "Initial.\n")]);
        let head = host.current_head();

        let commit_res =
            block_on(host.commit_file("doc.tmt", "Updated.\n", &head, "Update doc")).unwrap();
        assert_eq!(commit_res.revision.as_str(), "rev-1");
        assert_eq!(host.current_head().as_str(), "rev-1");
        assert_eq!(
            host.get_current_file("doc.tmt"),
            Some("Updated.\n".to_string())
        );
        assert_eq!(
            host.get_file_at(&Revision::new("rev-0"), "doc.tmt"),
            Some("Initial.\n".to_string())
        );
        assert_eq!(
            host.last_commit_message(),
            Some("Update doc".to_string())
        );
        assert_eq!(
            host.commit_message(&commit_res.revision),
            Some("Update doc".to_string())
        );
    }

    #[test]
    fn commit_fails_on_branch_moved() {
        let host = MemoryHost::with_files([("doc.tmt", "Initial.\n")]);
        let stale_head = host.current_head();

        // Simulate another party committing
        let new_rev = host.simulate_remote_commit("doc.tmt", Some("By someone else.\n"));
        assert_eq!(new_rev.as_str(), "rev-1");

        // Attempt commit with stale head
        let err =
            block_on(host.commit_file("doc.tmt", "My edit.\n", &stale_head, "My edit")).unwrap_err();
        match err {
            HostError::BranchMoved { expected, actual } => {
                assert_eq!(expected, stale_head);
                assert_eq!(actual.as_str(), "rev-1");
            }
            other => panic!("expected BranchMoved, got {other:?}"),
        }
    }

    #[test]
    fn simulate_branch_moved_countdown() {
        let host = MemoryHost::with_files([("doc.tmt", "Initial.\n")]);
        let head = host.current_head();

        host.simulate_branch_moved_times(1);

        let err = block_on(host.commit_file("doc.tmt", "My edit.\n", &head, "My edit")).unwrap_err();
        assert!(matches!(err, HostError::BranchMoved { .. }));

        // Second attempt with current head should now succeed because countdown reached 0
        let current_head = host.current_head();
        let res = block_on(host.commit_file("doc.tmt", "My edit.\n", &current_head, "Retry edit"));
        assert!(res.is_ok());
        assert_eq!(
            host.last_commit_message(),
            Some("Retry edit".to_string())
        );
    }

    #[test]
    fn simulate_delete() {
        let host = MemoryHost::with_files([("doc.tmt", "Initial.\n")]);
        assert!(host.get_current_file("doc.tmt").is_some());

        host.simulate_remote_commit("doc.tmt", None);
        assert!(host.get_current_file("doc.tmt").is_none());
    }
}
