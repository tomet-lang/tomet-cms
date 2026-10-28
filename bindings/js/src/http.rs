//! `fetch`-backed [`HttpClient`] implementation using `js-sys` and `wasm-bindgen-futures`.

use std::fmt;
use tomet_cms_github::http::{HttpClient, HttpError, HttpRequest, HttpResponse};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// An [`HttpClient`] delegating requests to a JavaScript `fetch` function.
///
/// Defaults to `globalThis.fetch`, but callers may provide a custom `fetch`-compatible
/// function (e.g. for testing, mocks, or interception).
#[derive(Clone, Default)]
pub struct JsHttpClient {
    custom_fetch: Option<js_sys::Function>,
}

impl JsHttpClient {
    /// Creates a new `JsHttpClient`. If `custom_fetch` is `None`, `globalThis.fetch` is used.
    pub fn new(custom_fetch: Option<js_sys::Function>) -> Self {
        Self { custom_fetch }
    }
}

impl fmt::Debug for JsHttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsHttpClient")
            .field("has_custom_fetch", &self.custom_fetch.is_some())
            .finish()
    }
}

impl HttpClient for JsHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let fetch_fn = match &self.custom_fetch {
            Some(f) => f.clone(),
            None => {
                let global = js_sys::global();
                let fetch_val = js_sys::Reflect::get(&global, &JsValue::from_str("fetch"))
                    .map_err(|e| HttpError::Other(format_js_error("failed to get global fetch", &e)))?;
                if !fetch_val.is_function() {
                    return Err(HttpError::Other("global fetch is not a function".to_string()));
                }
                fetch_val.unchecked_into::<js_sys::Function>()
            }
        };

        let init = js_sys::Object::new();
        js_sys::Reflect::set(
            &init,
            &JsValue::from_str("method"),
            &JsValue::from_str(request.method.as_str()),
        )
        .map_err(|e| HttpError::Other(format_js_error("failed to set method", &e)))?;

        let headers_obj = js_sys::Object::new();
        for (name, value) in &request.headers {
            js_sys::Reflect::set(
                &headers_obj,
                &JsValue::from_str(name),
                &JsValue::from_str(value),
            )
            .map_err(|e| HttpError::Other(format_js_error("failed to set header", &e)))?;
        }
        js_sys::Reflect::set(&init, &JsValue::from_str("headers"), &headers_obj)
            .map_err(|e| HttpError::Other(format_js_error("failed to set headers", &e)))?;

        if !request.body.is_empty() {
            let uint8 = js_sys::Uint8Array::from(request.body.as_slice());
            js_sys::Reflect::set(&init, &JsValue::from_str("body"), &uint8)
                .map_err(|e| HttpError::Other(format_js_error("failed to set body", &e)))?;
        }

        let promise_val = fetch_fn
            .call2(
                &js_sys::global(),
                &JsValue::from_str(&request.url),
                &init,
            )
            .map_err(|e| HttpError::Network(format_js_error("fetch call threw", &e)))?;

        let promise: js_sys::Promise = promise_val
            .dyn_into()
            .map_err(|_| HttpError::Network("fetch did not return a Promise".to_string()))?;

        let response_val = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|e| HttpError::Network(format_js_error("fetch rejected", &e)))?;

        let status_val = js_sys::Reflect::get(&response_val, &JsValue::from_str("status"))
            .map_err(|e| HttpError::Other(format_js_error("failed to get response.status", &e)))?;
        let status = status_val
            .as_f64()
            .ok_or_else(|| HttpError::Other("response.status is not a number".to_string()))?
            as u16;

        let mut headers_vec = Vec::new();
        if let Ok(headers_val) = js_sys::Reflect::get(&response_val, &JsValue::from_str("headers"))
            && !headers_val.is_undefined()
            && !headers_val.is_null()
        {
            if let Ok(entries_fn_val) =
                js_sys::Reflect::get(&headers_val, &JsValue::from_str("entries"))
                && entries_fn_val.is_function()
            {
                let entries_fn: js_sys::Function = entries_fn_val.unchecked_into();
                if let Ok(iterator) = entries_fn.call0(&headers_val) {
                    let array = js_sys::Array::from(&iterator);
                    for item in array.iter() {
                        let pair = js_sys::Array::from(&item);
                        if pair.length() >= 2 {
                            let key = pair.get(0).as_string().unwrap_or_default();
                            let val = pair.get(1).as_string().unwrap_or_default();
                            headers_vec.push((key, val));
                        }
                    }
                }
            } else if let Ok(keys) = js_sys::Reflect::own_keys(&headers_val) {
                for key_val in keys.iter() {
                    if let Some(key) = key_val.as_string()
                        && let Ok(val) = js_sys::Reflect::get(&headers_val, &key_val)
                        && let Some(v_str) = val.as_string()
                    {
                        headers_vec.push((key, v_str));
                    }
                }
            }
        }

        let array_buffer_fn_val = js_sys::Reflect::get(&response_val, &JsValue::from_str("arrayBuffer"))
            .map_err(|e| HttpError::Other(format_js_error("failed to get arrayBuffer function", &e)))?;
        let body_bytes = if array_buffer_fn_val.is_function() {
            let array_buffer_fn: js_sys::Function = array_buffer_fn_val.unchecked_into();
            let ab_promise: js_sys::Promise = array_buffer_fn
                .call0(&response_val)
                .map_err(|e| HttpError::Other(format_js_error("arrayBuffer call threw", &e)))?
                .dyn_into()
                .map_err(|_| HttpError::Other("arrayBuffer did not return a Promise".to_string()))?;
            let ab_val = wasm_bindgen_futures::JsFuture::from(ab_promise)
                .await
                .map_err(|e| HttpError::Other(format_js_error("arrayBuffer rejected", &e)))?;
            let uint8 = js_sys::Uint8Array::new(&ab_val);
            uint8.to_vec()
        } else if let Ok(text_fn_val) = js_sys::Reflect::get(&response_val, &JsValue::from_str("text")) {
            if text_fn_val.is_function() {
                let text_fn: js_sys::Function = text_fn_val.unchecked_into();
                let t_promise: js_sys::Promise = text_fn
                    .call0(&response_val)
                    .map_err(|e| HttpError::Other(format_js_error("text call threw", &e)))?
                    .dyn_into()
                    .map_err(|_| HttpError::Other("text did not return a Promise".to_string()))?;
                let t_val = wasm_bindgen_futures::JsFuture::from(t_promise)
                    .await
                    .map_err(|e| HttpError::Other(format_js_error("text rejected", &e)))?;
                t_val.as_string().unwrap_or_default().into_bytes()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let mut resp = HttpResponse::new(status, body_bytes);
        for (k, v) in headers_vec {
            resp = resp.with_header(k, v);
        }
        Ok(resp)
    }
}

fn format_js_error(prefix: &str, js_val: &JsValue) -> String {
    if let Ok(msg) = js_sys::Reflect::get(js_val, &JsValue::from_str("message"))
        && let Some(s) = msg.as_string()
    {
        return format!("{prefix}: {s}");
    }
    if let Some(s) = js_val.as_string() {
        return format!("{prefix}: {s}");
    }
    format!("{prefix}: {js_val:?}")
}
