//! Data transfer objects for JavaScript/TypeScript communication.

use serde::{Deserialize, Serialize};

/// Span byte range within a `.tmt` source string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsSpan {
    pub start: usize,
    pub end: usize,
}

/// Information about a detected `@conflict` marker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsConflictInfo {
    pub index: usize,
    /// Kind of conflict: `"block"`, `"inline"`, or `"scalar"`.
    pub kind: &'static str,
    pub span: JsSpan,
    pub a: String,
    pub b: String,
}

/// Warning generated during a save operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum JsSaveWarning {
    ModifyDeleteConflict { path: String, message: String },
}

impl From<tomet_cms::save::SaveWarning> for JsSaveWarning {
    fn from(w: tomet_cms::save::SaveWarning) -> Self {
        match w {
            tomet_cms::save::SaveWarning::ModifyDeleteConflict { path } => {
                JsSaveWarning::ModifyDeleteConflict {
                    message: format!(
                        "modify/delete conflict: remote deleted '{path}' while local edited it; kept local content"
                    ),
                    path,
                }
            }
        }
    }
}

/// Result returned after a successful save operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsSaveResult {
    pub revision: String,
    pub has_conflicts: bool,
    pub warnings: Vec<JsSaveWarning>,
}

/// Content returned when reading a file from a host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsReadFileResult {
    pub content: String,
    pub revision: String,
}

/// Options passed to `new GitHubCms(options)`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubCmsOptionsDto {
    pub owner: String,
    pub repo: String,
    pub branch: Option<String>,
    pub token: String,
    #[serde(alias = "baseUrl", alias = "base_url")]
    pub base_url: Option<String>,
}

/// Options passed to `createFile(req)`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileReqDto {
    pub path: String,
    pub content: String,
    pub message: String,
    #[serde(alias = "maxRetries", alias = "max_retries")]
    pub max_retries: Option<usize>,
}

/// Options passed to `editFile(req)`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFileReqDto {
    pub path: String,
    #[serde(alias = "baseRev", alias = "base_rev")]
    pub base_rev: String,
    pub content: String,
    pub message: String,
    #[serde(alias = "maxRetries", alias = "max_retries")]
    pub max_retries: Option<usize>,
}
