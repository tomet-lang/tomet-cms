//! GitHub implementation of [`Host`] using GraphQL and an injected [`HttpClient`].

use std::fmt;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tomet_cms::host::{CommitResult, FileContent, Host, HostError, Revision};

use crate::http::{HttpClient, HttpRequest, HttpResponse};

const DEFAULT_BASE_URL: &str = "https://api.github.com";

/// A [`Host`] implementation communicating with GitHub via GraphQL.
///
/// Authentication is performed via a personal access token passed to [`GitHubHost::new`].
/// All requests are routed through the injected [`HttpClient`].
///
/// The token is treated as secret and is never displayed in [`fmt::Debug`] representations,
/// logs, or error text.
pub struct GitHubHost<C> {
    http: C,
    owner: String,
    repo: String,
    branch: String,
    token: String,
    base_url: String,
}

impl<C> fmt::Debug for GitHubHost<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubHost")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("branch", &self.branch)
            .field("token", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl<C: HttpClient> GitHubHost<C> {
    /// Creates a new `GitHubHost` targeting `owner/repo` on `branch` using `token`.
    pub fn new(
        http: C,
        owner: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            http,
            owner: owner.into(),
            repo: repo.into(),
            branch: branch.into(),
            token: token.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Overrides the base URL (useful for GitHub Enterprise or mock endpoints).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Returns the target owner name.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Returns the target repository name.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Returns the target branch name.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Returns the qualified ref string for the target branch (e.g. `refs/heads/main`).
    fn qualified_branch(&self) -> String {
        if self.branch.starts_with("refs/") {
            self.branch.clone()
        } else {
            format!("refs/heads/{}", self.branch)
        }
    }

    async fn execute_graphql<V: Serialize, D: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: V,
    ) -> Result<D, HostError> {
        let endpoint = format!("{}/graphql", self.base_url.trim_end_matches('/'));
        let request_body = serde_json::to_vec(&GraphQLRequest { query, variables })
            .map_err(|e| HostError::Other(format!("failed to serialize GraphQL request: {e}")))?;

        let request = HttpRequest::post(endpoint)
            .with_header("Authorization", format!("Bearer {}", self.token))
            .with_header("Content-Type", "application/json")
            .with_header("User-Agent", "tomet-cms")
            .with_body(request_body);

        let response: HttpResponse = self
            .http
            .send(request)
            .await
            .map_err(|e| HostError::Other(format!("HTTP transport error: {e}")))?;

        match response.status {
            200 => {
                let parsed: GraphQLResponse<D> = serde_json::from_slice(&response.body)
                    .map_err(|e| {
                        HostError::Other(format!("failed to parse GraphQL response JSON: {e}"))
                    })?;

                if let Some(errors) = parsed.errors {
                    if !errors.is_empty() {
                        return Err(Self::map_graphql_errors(errors));
                    }
                }

                parsed.data.ok_or_else(|| {
                    HostError::Other("GraphQL response contained no data and no errors".to_string())
                })
            }
            401 => Err(HostError::Unauthorized),
            403 => {
                let body_str = String::from_utf8_lossy(&response.body);
                let rate_limit_remaining = response
                    .header("x-ratelimit-remaining")
                    .and_then(|v| v.parse::<u64>().ok());

                if rate_limit_remaining == Some(0)
                    || body_str.to_ascii_lowercase().contains("rate limit")
                {
                    Err(HostError::RateLimited)
                } else {
                    Err(HostError::Forbidden(body_str.trim().to_string()))
                }
            }
            429 => Err(HostError::RateLimited),
            status => {
                let body_str = String::from_utf8_lossy(&response.body);
                Err(HostError::Other(format!(
                    "GitHub API HTTP error {status}: {}",
                    body_str.trim()
                )))
            }
        }
    }

    fn map_graphql_errors(errors: Vec<GraphQLError>) -> HostError {
        for err in &errors {
            let lower = err.message.to_ascii_lowercase();
            if lower.contains("rate limit") {
                return HostError::RateLimited;
            }
            if lower.contains("bad credentials") {
                return HostError::Unauthorized;
            }
            if lower.contains("not accessible by personal access token")
                || lower.contains("permission")
            {
                return HostError::Forbidden(err.message.clone());
            }
        }

        let combined = errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        HostError::Other(format!("GraphQL error: {combined}"))
    }
}

// ---------------------------------------------------------------------------
// GraphQL Request & Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct GraphQLRequest<'a, V> {
    query: &'a str,
    variables: V,
}

#[derive(Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize, Debug, Clone)]
struct GraphQLError {
    message: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    error_type: Option<String>,
}

// ---------------------------------------------------------------------------
// get_head GraphQL types
// ---------------------------------------------------------------------------

const GET_HEAD_QUERY: &str = r#"
query GetHead($owner: String!, $repo: String!, $qualifiedBranch: String!) {
  repository(owner: $owner, name: $repo) {
    ref(qualifiedName: $qualifiedBranch) {
      target {
        oid
      }
    }
  }
}
"#;

#[derive(Serialize)]
struct GetHeadVariables<'a> {
    owner: &'a str,
    repo: &'a str,
    #[serde(rename = "qualifiedBranch")]
    qualified_branch: &'a str,
}

#[derive(Deserialize)]
struct GetHeadData {
    repository: Option<GetHeadRepo>,
}

#[derive(Deserialize)]
struct GetHeadRepo {
    #[serde(rename = "ref")]
    git_ref: Option<GetHeadRef>,
}

#[derive(Deserialize)]
struct GetHeadRef {
    target: Option<GetHeadTarget>,
}

#[derive(Deserialize)]
struct GetHeadTarget {
    oid: Option<String>,
}

// ---------------------------------------------------------------------------
// read_file GraphQL types
// ---------------------------------------------------------------------------

const READ_FILE_QUERY: &str = r#"
query ReadFile($owner: String!, $repo: String!, $expression: String!) {
  repository(owner: $owner, name: $repo) {
    object(expression: $expression) {
      __typename
      ... on Blob {
        isBinary
        isTruncated
        text
        byteSize
      }
    }
  }
}
"#;

#[derive(Serialize)]
struct ReadFileVariables<'a> {
    owner: &'a str,
    repo: &'a str,
    expression: String,
}

#[derive(Deserialize)]
struct ReadFileData {
    repository: Option<ReadFileRepo>,
}

#[derive(Deserialize)]
struct ReadFileRepo {
    object: Option<BlobObject>,
}

#[derive(Deserialize)]
struct BlobObject {
    #[serde(rename = "isBinary")]
    is_binary: Option<bool>,
    #[serde(rename = "isTruncated")]
    is_truncated: Option<bool>,
    text: Option<String>,
}

// ---------------------------------------------------------------------------
// commit_file GraphQL types
// ---------------------------------------------------------------------------

const COMMIT_FILE_MUTATION: &str = r#"
mutation CommitFile($input: CreateCommitOnBranchInput!) {
  createCommitOnBranch(input: $input) {
    commit {
      oid
    }
  }
}
"#;

#[derive(Serialize)]
struct CommitFileVariables<'a> {
    input: CreateCommitOnBranchInput<'a>,
}

#[derive(Serialize)]
struct CreateCommitOnBranchInput<'a> {
    branch: CommittableBranch<'a>,
    #[serde(rename = "expectedHeadOid")]
    expected_head_oid: &'a str,
    message: CommitMessageHeadline<'a>,
    #[serde(rename = "fileChanges")]
    file_changes: FileChanges<'a>,
}

#[derive(Serialize)]
struct CommittableBranch<'a> {
    #[serde(rename = "repositoryNameWithOwner")]
    repository_name_with_owner: String,
    #[serde(rename = "branchName")]
    branch_name: &'a str,
}

#[derive(Serialize)]
struct CommitMessageHeadline<'a> {
    headline: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

impl<'a> CommitMessageHeadline<'a> {
    fn from_message(message: &'a str) -> Self {
        match message.split_once('\n') {
            Some((headline, rest)) => {
                let body = rest.trim_start_matches(['\r', '\n']);
                Self {
                    headline: headline.trim_end_matches('\r'),
                    body: if body.is_empty() { None } else { Some(body) },
                }
            }
            None => Self {
                headline: message,
                body: None,
            },
        }
    }
}

#[derive(Serialize)]
struct FileChanges<'a> {
    additions: Vec<FileAddition<'a>>,
}

#[derive(Serialize)]
struct FileAddition<'a> {
    path: &'a str,
    contents: String,
}

#[derive(Deserialize)]
struct CommitFileData {
    #[serde(rename = "createCommitOnBranch")]
    create_commit_on_branch: Option<CreateCommitOnBranchPayload>,
}

#[derive(Deserialize)]
struct CreateCommitOnBranchPayload {
    commit: Option<CommitTarget>,
}

#[derive(Deserialize)]
struct CommitTarget {
    oid: Option<String>,
}

// ---------------------------------------------------------------------------
// Host implementation
// ---------------------------------------------------------------------------

impl<C: HttpClient> Host for GitHubHost<C> {
    async fn get_head(&self) -> Result<Revision, HostError> {
        let qualified_branch = self.qualified_branch();
        let variables = GetHeadVariables {
            owner: &self.owner,
            repo: &self.repo,
            qualified_branch: &qualified_branch,
        };

        let data: GetHeadData = self.execute_graphql(GET_HEAD_QUERY, variables).await?;

        let repo = data.repository.ok_or_else(|| {
            HostError::Other(format!(
                "repository '{}/{}' not found",
                self.owner, self.repo
            ))
        })?;

        let git_ref = repo
            .git_ref
            .ok_or_else(|| HostError::RevisionNotFound(Revision::new(&self.branch)))?;

        let oid = git_ref
            .target
            .and_then(|t| t.oid)
            .ok_or_else(|| HostError::RevisionNotFound(Revision::new(&self.branch)))?;

        Ok(Revision::new(oid))
    }

    async fn read_file(
        &self,
        path: &str,
        rev: &Revision,
    ) -> Result<Option<FileContent>, HostError> {
        let expression = format!("{}:{}", rev.as_str(), path);
        let variables = ReadFileVariables {
            owner: &self.owner,
            repo: &self.repo,
            expression,
        };

        let data: ReadFileData = self.execute_graphql(READ_FILE_QUERY, variables).await?;

        let repo = data.repository.ok_or_else(|| {
            HostError::Other(format!(
                "repository '{}/{}' not found",
                self.owner, self.repo
            ))
        })?;

        match repo.object {
            None => Ok(None),
            Some(blob) => {
                if blob.is_binary == Some(true) {
                    return Err(HostError::NonTextBlob {
                        path: path.to_string(),
                    });
                }
                if blob.is_truncated == Some(true) {
                    return Err(HostError::Other(format!(
                        "file blob '{path}' at revision {rev} is truncated (exceeds GraphQL size limit)"
                    )));
                }
                match blob.text {
                    Some(text) => Ok(Some(FileContent::new(text, rev.clone()))),
                    None => Ok(None),
                }
            }
        }
    }

    async fn commit_file(
        &self,
        path: &str,
        content: &str,
        expected_head: &Revision,
        message: &str,
    ) -> Result<CommitResult, HostError> {
        let base64_content = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());

        let variables = CommitFileVariables {
            input: CreateCommitOnBranchInput {
                branch: CommittableBranch {
                    repository_name_with_owner: format!("{}/{}", self.owner, self.repo),
                    branch_name: &self.branch,
                },
                expected_head_oid: expected_head.as_str(),
                message: CommitMessageHeadline::from_message(message),
                file_changes: FileChanges {
                    additions: vec![FileAddition {
                        path,
                        contents: base64_content,
                    }],
                },
            },
        };

        let commit_res: Result<CommitFileData, HostError> =
            self.execute_graphql(COMMIT_FILE_MUTATION, variables).await;

        match commit_res {
            Ok(data) => {
                let oid = data
                    .create_commit_on_branch
                    .and_then(|p| p.commit)
                    .and_then(|c| c.oid)
                    .ok_or_else(|| {
                        HostError::Other(
                            "createCommitOnBranch returned empty commit payload".to_string(),
                        )
                    })?;
                Ok(CommitResult::new(oid))
            }
            Err(original_err) => {
                // On any commit failure, re-fetch head. If head has changed, the cause was
                // a concurrent branch movement, so report BranchMoved rather than trusting
                // error message wording.
                if let Ok(actual_head) = self.get_head().await {
                    if actual_head != *expected_head {
                        return Err(HostError::BranchMoved {
                            expected: expected_head.clone(),
                            actual: actual_head,
                        });
                    }
                }
                Err(original_err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FakeHttpClient;
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

    fn test_host(client: FakeHttpClient) -> GitHubHost<FakeHttpClient> {
        GitHubHost::new(client, "test-owner", "test-repo", "main", "secret_token_123")
    }

    #[test]
    fn token_is_redacted_in_debug_output() {
        let client = FakeHttpClient::new();
        let host = test_host(client);
        let debug_str = format!("{host:?}");
        assert!(!debug_str.contains("secret_token_123"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn get_head_success() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "ref": {
                            "target": {
                                "oid": "1111111111111111111111111111111111111111"
                            }
                        }
                    }
                }
            }),
        );

        let host = test_host(client.clone());
        let rev = block_on(host.get_head()).unwrap();
        assert_eq!(rev.as_str(), "1111111111111111111111111111111111111111");

        let last_req = client.last_request().unwrap();
        assert_eq!(last_req.url, "https://api.github.com/graphql");
        assert_eq!(last_req.header("authorization"), Some("Bearer secret_token_123"));
    }

    #[test]
    fn get_head_missing_branch_returns_revision_not_found() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "ref": null
                    }
                }
            }),
        );

        let host = test_host(client);
        let err = block_on(host.get_head()).unwrap_err();
        assert!(matches!(err, HostError::RevisionNotFound(rev) if rev.as_str() == "main"));
    }

    #[test]
    fn read_file_success() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "object": {
                            "isBinary": false,
                            "isTruncated": false,
                            "text": "Hello world from GitHub!\n",
                            "byteSize": 25
                        }
                    }
                }
            }),
        );

        let host = test_host(client);
        let rev = Revision::new("rev-1");
        let content = block_on(host.read_file("doc.tmt", &rev)).unwrap().unwrap();
        assert_eq!(content.content, "Hello world from GitHub!\n");
        assert_eq!(content.revision, rev);
    }

    #[test]
    fn read_file_absent_returns_ok_none() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "object": null
                    }
                }
            }),
        );

        let host = test_host(client);
        let rev = Revision::new("rev-1");
        let res = block_on(host.read_file("missing.tmt", &rev)).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn read_file_binary_returns_non_text_blob_error() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "object": {
                            "isBinary": true,
                            "isTruncated": false,
                            "text": null,
                            "byteSize": 1024
                        }
                    }
                }
            }),
        );

        let host = test_host(client);
        let rev = Revision::new("rev-1");
        let err = block_on(host.read_file("image.png", &rev)).unwrap_err();
        assert!(matches!(err, HostError::NonTextBlob { path } if path == "image.png"));
    }

    #[test]
    fn read_file_truncated_returns_typed_error() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "object": {
                            "isBinary": false,
                            "isTruncated": true,
                            "text": "Partial...",
                            "byteSize": 10_000_000
                        }
                    }
                }
            }),
        );

        let host = test_host(client);
        let rev = Revision::new("rev-1");
        let err = block_on(host.read_file("huge.tmt", &rev)).unwrap_err();
        assert!(matches!(err, HostError::Other(msg) if msg.contains("truncated")));
    }

    #[test]
    fn commit_file_success() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "createCommitOnBranch": {
                        "commit": {
                            "oid": "2222222222222222222222222222222222222222"
                        }
                    }
                }
            }),
        );

        let host = test_host(client.clone());
        let expected_head = Revision::new("1111111111111111111111111111111111111111");
        let res = block_on(host.commit_file(
            "doc.tmt",
            "Updated text.\n",
            &expected_head,
            "Update doc",
        ))
        .unwrap();

        assert_eq!(res.revision.as_str(), "2222222222222222222222222222222222222222");

        let last_req = client.last_request().unwrap();
        let body_str = String::from_utf8(last_req.body).unwrap();
        // Base64 of "Updated text.\n" is "VXBkYXRlZCB0ZXh0Lgo="
        assert!(body_str.contains("VXBkYXRlZCB0ZXh0Lgo="));
        assert!(body_str.contains("Update doc"));
    }

    #[test]
    fn token_rejected_401_maps_to_unauthorized() {
        let client = FakeHttpClient::new();
        client.queue_response(
            HttpResponse::new(401, b"{\"message\": \"Bad credentials\"}")
                .with_header("content-type", "application/json"),
        );

        let host = test_host(client);
        let err = block_on(host.get_head()).unwrap_err();
        assert!(matches!(err, HostError::Unauthorized));
    }

    #[test]
    fn rate_limited_403_maps_to_rate_limited() {
        let client = FakeHttpClient::new();
        client.queue_response(
            HttpResponse::new(403, b"API rate limit exceeded for user")
                .with_header("x-ratelimit-remaining", "0"),
        );

        let host = test_host(client);
        let err = block_on(host.get_head()).unwrap_err();
        assert!(matches!(err, HostError::RateLimited));
    }

    #[test]
    fn forbidden_403_maps_to_forbidden() {
        let client = FakeHttpClient::new();
        client.queue_response(
            HttpResponse::new(403, b"Resource not accessible by personal access token")
                .with_header("content-type", "application/json"),
        );

        let host = test_host(client);
        let err = block_on(host.get_head()).unwrap_err();
        assert!(matches!(err, HostError::Forbidden(msg) if msg.contains("not accessible")));
    }

    #[test]
    fn graphql_errors_array_with_200_maps_properly() {
        let client = FakeHttpClient::new();
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": null,
                "errors": [
                    {
                        "message": "Resource not accessible by personal access token",
                        "type": "FORBIDDEN"
                    }
                ]
            }),
        );

        let host = test_host(client);
        let err = block_on(host.get_head()).unwrap_err();
        assert!(matches!(err, HostError::Forbidden(msg) if msg.contains("not accessible")));
    }

    #[test]
    fn commit_fails_and_branch_moved_detected() {
        let client = FakeHttpClient::new();
        let expected_head = Revision::new("1111111111111111111111111111111111111111");

        // 1. Commit fails (e.g. GraphQL error from GitHub)
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": null,
                "errors": [
                    {
                        "message": "Head branch was modified; expected 1111 but actual was 3333"
                    }
                ]
            }),
        );

        // 2. Follow-up get_head query returns new head
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "ref": {
                            "target": {
                                "oid": "3333333333333333333333333333333333333333"
                            }
                        }
                    }
                }
            }),
        );

        let host = test_host(client);
        let err = block_on(host.commit_file(
            "doc.tmt",
            "Text",
            &expected_head,
            "Commit msg",
        ))
        .unwrap_err();

        match err {
            HostError::BranchMoved { expected, actual } => {
                assert_eq!(expected, expected_head);
                assert_eq!(actual.as_str(), "3333333333333333333333333333333333333333");
            }
            other => panic!("expected BranchMoved, got {other:?}"),
        }
    }

    #[test]
    fn commit_fails_and_head_unchanged_maps_original_error() {
        let client = FakeHttpClient::new();
        let expected_head = Revision::new("1111111111111111111111111111111111111111");

        // 1. Commit fails with 403 Forbidden
        client.queue_response(
            HttpResponse::new(403, b"Write access denied to repository")
                .with_header("content-type", "application/json"),
        );

        // 2. Follow-up get_head query returns SAME head
        client.queue_json_response(
            200,
            &serde_json::json!({
                "data": {
                    "repository": {
                        "ref": {
                            "target": {
                                "oid": "1111111111111111111111111111111111111111"
                            }
                        }
                    }
                }
            }),
        );

        let host = test_host(client);
        let err = block_on(host.commit_file(
            "doc.tmt",
            "Text",
            &expected_head,
            "Commit msg",
        ))
        .unwrap_err();

        // Must map the original error, NOT BranchMoved!
        assert!(matches!(err, HostError::Forbidden(msg) if msg.contains("Write access denied")));
    }
}
