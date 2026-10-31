//! JavaScript/TypeScript WebAssembly bindings for `tomet-cms` and `tomet-cms-github`.

pub mod error;
pub mod http;
pub mod types;

use std::fmt;

use error::{create_js_error, map_conflict_error, map_host_error, map_save_error};
use http::JsHttpClient;
use tomet_cms::host::{Host, Revision};
use tomet_cms::tomet::merge::{self, ConflictKind, ConflictResolution};
use tomet_cms_github::GitHubHost;
use types::{
    CreateFileReqDto, EditFileReqDto, GitHubCmsOptionsDto, JsConflictInfo, JsReadFileResult,
    JsSaveResult, JsSaveWarning, JsSpan,
};
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// Conflict functions (pure, synchronous, no host required)
// ---------------------------------------------------------------------------

/// Finds and lists all `@conflict` markers in `.tmt` document source.
///
/// Returns an array of [`JsConflictInfo`] objects in top-down document order. `span` is a
/// range of BYTE offsets into the UTF-8 source, and `a`/`b` are the sides as written in it.
/// Throws an `Error` with `.code = "ConflictParseError"` if a marker cannot be read: an
/// unreadable marker is never skipped.
#[wasm_bindgen(js_name = listConflicts)]
pub fn list_conflicts(source: &str) -> Result<JsValue, JsValue> {
    let conflicts = merge::find_conflicts(source).map_err(map_conflict_error)?;
    let js_conflicts: Vec<JsConflictInfo> = conflicts
        .into_iter()
        .map(|c| JsConflictInfo {
            index: c.index,
            kind: match c.kind {
                ConflictKind::Block => "block",
                ConflictKind::Inline => "inline",
                ConflictKind::Value => "scalar",
            },
            span: JsSpan {
                start: c.span.start,
                end: c.span.end,
            },
            a: c.a,
            b: c.b,
        })
        .collect();

    serde_wasm_bindgen::to_value(&js_conflicts)
        .map_err(|e| create_js_error("SerializationError", &e.to_string()))
}

/// Resolves a single `@conflict` marker at `index` to the chosen `side` (`"a"` or `"b"`).
///
/// Returns the updated document source text with the marker replaced by the chosen
/// side, written as it stood in the marker. Every byte outside the marker is kept; a block
/// marker also takes the line break after it with it.
///
/// Throws an `Error` with:
/// - `.code = "IndexOutOfBounds"` if `index` does not exist.
/// - `.code = "InvalidSide"` if `side` is not `"a"` or `"b"`.
/// - `.code = "ConflictParseError"` if a marker in the document cannot be read.
#[wasm_bindgen(js_name = resolveConflict)]
pub fn resolve_conflict(source: &str, index: usize, side: &str) -> Result<String, JsValue> {
    let resolution = match side {
        "a" | "A" => ConflictResolution::A,
        "b" | "B" => ConflictResolution::B,
        _ => return Err(create_js_error("InvalidSide", "conflict side must be 'a' or 'b'")),
    };

    merge::resolve_single(source, index, resolution).map_err(map_conflict_error)
}

// ---------------------------------------------------------------------------
// GitHubCms client
// ---------------------------------------------------------------------------

/// High-level content management client for GitHub repositories.
#[wasm_bindgen]
pub struct GitHubCms {
    host: GitHubHost<JsHttpClient>,
}

impl fmt::Debug for GitHubCms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubCms")
            .field("host", &self.host)
            .finish()
    }
}

#[wasm_bindgen]
impl GitHubCms {
    /// Creates a new `GitHubCms` client.
    ///
    /// `options` is an object: `{ owner, repo, branch?, token, baseUrl?, fetch? }`.
    /// `branch` defaults to `"main"`.
    /// `fetch` defaults to `globalThis.fetch`.
    #[wasm_bindgen(constructor)]
    pub fn new(options: JsValue) -> Result<GitHubCms, JsValue> {
        let dto: GitHubCmsOptionsDto = serde_wasm_bindgen::from_value(options.clone())
            .map_err(|e| create_js_error("InvalidOptions", &format!("invalid options: {e}")))?;

        let custom_fetch = if let Ok(f) = js_sys::Reflect::get(&options, &JsValue::from_str("fetch")) {
            if f.is_function() {
                Some(f.unchecked_into::<js_sys::Function>())
            } else {
                None
            }
        } else {
            None
        };

        let branch = dto.branch.unwrap_or_else(|| "main".to_string());
        let http = JsHttpClient::new(custom_fetch);
        let mut host = GitHubHost::new(http, dto.owner, dto.repo, branch, dto.token);
        if let Some(base_url) = dto.base_url {
            host = host.with_base_url(base_url);
        }

        Ok(GitHubCms { host })
    }

    /// Fetches the target branch's current head commit revision OID.
    #[wasm_bindgen(js_name = getHead)]
    pub async fn get_head(&self) -> Result<String, JsValue> {
        let rev = self.host.get_head().await.map_err(map_host_error)?;
        Ok(rev.to_string())
    }

    /// Reads a file at `path` from revision `rev`.
    ///
    /// Returns `{ content: string, revision: string }` if present, or `null` if absent.
    #[wasm_bindgen(js_name = readFile)]
    pub async fn read_file(&self, path: &str, rev: &str) -> Result<JsValue, JsValue> {
        let revision = Revision::new(rev);
        let file = self.host.read_file(path, &revision).await.map_err(map_host_error)?;
        match file {
            Some(f) => {
                let res = JsReadFileResult {
                    content: f.content,
                    revision: f.revision.to_string(),
                };
                serde_wasm_bindgen::to_value(&res)
                    .map_err(|e| create_js_error("SerializationError", &e.to_string()))
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Creates a fresh document at `path`.
    ///
    /// If the file already exists at HEAD, rejects with `err.code === "AlreadyExists"`.
    #[wasm_bindgen(js_name = createFile)]
    pub async fn create_file(&self, req: JsValue) -> Result<JsValue, JsValue> {
        let dto: CreateFileReqDto = serde_wasm_bindgen::from_value(req).map_err(|e| {
            create_js_error("InvalidRequest", &format!("invalid createFile request: {e}"))
        })?;

        let request = tomet_cms::save::SaveRequest::create(&dto.path, &dto.content, &dto.message);
        let options = tomet_cms::save::SaveOptions {
            max_retries: dto.max_retries.unwrap_or(tomet_cms::save::DEFAULT_MAX_RETRIES),
        };

        let result = tomet_cms::save::save_with_options(&self.host, request, options)
            .await
            .map_err(map_save_error)?;

        let js_result = JsSaveResult {
            revision: result.commit.revision.to_string(),
            has_conflicts: result.has_conflicts,
            warnings: result.warnings.into_iter().map(JsSaveWarning::from).collect(),
        };

        serde_wasm_bindgen::to_value(&js_result)
            .map_err(|e| create_js_error("SerializationError", &e.to_string()))
    }

    /// Edits an existing document at `path` based on `baseRev`.
    ///
    /// Performs 3-way AST merge with remote changes. On concurrent branch movement,
    /// retries up to `maxRetries` (default 3).
    #[wasm_bindgen(js_name = editFile)]
    pub async fn edit_file(&self, req: JsValue) -> Result<JsValue, JsValue> {
        let dto: EditFileReqDto = serde_wasm_bindgen::from_value(req).map_err(|e| {
            create_js_error("InvalidRequest", &format!("invalid editFile request: {e}"))
        })?;

        let base_rev = Revision::new(&dto.base_rev);
        let request =
            tomet_cms::save::SaveRequest::edit(&dto.path, &base_rev, &dto.content, &dto.message);
        let options = tomet_cms::save::SaveOptions {
            max_retries: dto.max_retries.unwrap_or(tomet_cms::save::DEFAULT_MAX_RETRIES),
        };

        let result = tomet_cms::save::save_with_options(&self.host, request, options)
            .await
            .map_err(map_save_error)?;

        let js_result = JsSaveResult {
            revision: result.commit.revision.to_string(),
            has_conflicts: result.has_conflicts,
            warnings: result.warnings.into_iter().map(JsSaveWarning::from).collect(),
        };

        serde_wasm_bindgen::to_value(&js_result)
            .map_err(|e| create_js_error("SerializationError", &e.to_string()))
    }
}
