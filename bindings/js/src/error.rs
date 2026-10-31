//! Structured error translation between Rust errors and JavaScript `Error` objects.

use tomet_cms::host::HostError;
use tomet_cms::save::SaveError;
use tomet_cms::tomet::merge::ConflictError;
use wasm_bindgen::prelude::*;

/// Constructs a JavaScript `Error` instance with an attached `.code: string` property.
pub fn create_js_error(code: &str, message: &str) -> JsValue {
    let err = js_sys::Error::new(message);
    let _ = js_sys::Reflect::set(&err, &JsValue::from_str("code"), &JsValue::from_str(code));
    JsValue::from(err)
}

/// Maps [`ConflictError`] to a JavaScript error object with an appropriate `.code`.
pub fn map_conflict_error(err: ConflictError) -> JsValue {
    match err {
        ConflictError::Malformed { offset, message } => create_js_error(
            "ConflictParseError",
            &format!("invalid conflict marker at byte {offset}: {message}"),
        ),
        ConflictError::IndexOutOfBounds { index, count } => create_js_error(
            "IndexOutOfBounds",
            &format!("conflict index {index} out of bounds (found {count} conflicts)"),
        ),
    }
}

/// Maps [`HostError`] to a JavaScript error object with an appropriate `.code`.
pub fn map_host_error(err: HostError) -> JsValue {
    match err {
        HostError::Unauthorized => {
            create_js_error("Unauthorized", "unauthorized: invalid or expired GitHub token")
        }
        HostError::Forbidden(msg) => create_js_error("Forbidden", &msg),
        HostError::RateLimited => {
            create_js_error("RateLimited", "GitHub API rate limit exceeded")
        }
        HostError::NonTextBlob { path } => {
            create_js_error("NonTextBlob", &format!("binary or non-text blob: {path}"))
        }
        HostError::BranchMoved { .. } => {
            create_js_error("BranchMoved", "remote branch moved concurrently")
        }
        HostError::RevisionNotFound(rev) => {
            create_js_error("RevisionNotFound", &format!("revision not found: {rev}"))
        }
        HostError::Other(msg) => create_js_error("HostOther", &msg),
    }
}

/// Maps [`SaveError`] to a JavaScript error object with an appropriate `.code`.
pub fn map_save_error(err: SaveError) -> JsValue {
    match err {
        SaveError::NonTmtPath(path) => {
            create_js_error("NonTmtPath", &format!("path must end with .tmt: {path}"))
        }
        SaveError::AlreadyExists { path } => {
            create_js_error("AlreadyExists", &format!("file already exists: {path}"))
        }
        SaveError::MaxRetriesExceeded { attempts } => create_js_error(
            "MaxRetriesExceeded",
            &format!("max retries exceeded ({attempts} attempts)"),
        ),
        SaveError::ParseError(msg) => create_js_error("ParseError", &msg),
        SaveError::Host(h) => map_host_error(h),
    }
}
