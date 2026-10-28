//! Injected HTTP client abstraction and test fake.
//!
//! Provides the [`HttpClient`] trait used by [`crate::GitHubHost`] to communicate with
//! remote hosts without binding this crate to any specific HTTP library or runtime.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

/// An HTTP request method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Other(String),
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
            Self::Head => write!(f, "HEAD"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}

/// An outgoing HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Creates a new request with no headers or body.
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Convenience constructor for a POST request.
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    /// Adds a header to the request.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the raw request body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Looks up a header value by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// An incoming HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Creates a new response with status and body.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Adds a header to the response.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Looks up a response header value by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Errors occurring at the HTTP transport layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Network connection failed, unreachable host, or timeout.
    Network(String),
    /// Request was aborted.
    Aborted,
    /// Other client-side transport failure.
    Other(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Aborted => write!(f, "request aborted"),
            Self::Other(msg) => write!(f, "http client error: {msg}"),
        }
    }
}

impl std::error::Error for HttpError {}

/// Abstract HTTP transport client.
///
/// # Concurrency & Single-Threading Contract
/// The futures returned by [`HttpClient`] methods are deliberately **not** bounded by `Send`.
/// `tomet-cms` and `tomet-cms-github` are designed to run in browser WebAssembly
/// (`wasm32-unknown-unknown`), which is single-threaded. In browser environments,
/// this trait is backed by JavaScript's `window.fetch`. Callers are responsible for
/// driving the returned futures on their local task executor.
#[allow(async_fn_in_trait)]
pub trait HttpClient {
    /// Sends an HTTP request and asynchronously returns the response or a transport error.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
}

impl<T: HttpClient> HttpClient for &T {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        (**self).send(request).await
    }
}

impl<T: HttpClient> HttpClient for Rc<T> {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        (**self).send(request).await
    }
}

/// A fake HTTP client for testing and offline simulations.
///
/// Records all sent requests and replays queued responses in FIFO order.
/// Wraps its internal state in an [`Rc`]/[`RefCell`], so cloning a [`FakeHttpClient`]
/// produces a handle sharing the same recording and response queue.
#[derive(Debug, Clone)]
pub struct FakeHttpClient {
    state: Rc<RefCell<FakeHttpClientState>>,
}

#[derive(Debug, Default)]
struct FakeHttpClientState {
    requests: Vec<HttpRequest>,
    responses: VecDeque<Result<HttpResponse, HttpError>>,
}

impl Default for FakeHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHttpClient {
    /// Creates a new empty `FakeHttpClient`.
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(FakeHttpClientState::default())),
        }
    }

    /// Queues a response to be returned by a subsequent [`HttpClient::send`].
    pub fn queue_response(&self, response: HttpResponse) {
        self.state.borrow_mut().responses.push_back(Ok(response));
    }

    /// Queues a JSON response with status code and serialized object.
    pub fn queue_json_response<T: serde::Serialize>(&self, status: u16, data: &T) {
        let body = serde_json::to_vec(data).expect("failed to serialize test json response");
        let resp = HttpResponse::new(status, body).with_header("content-type", "application/json");
        self.queue_response(resp);
    }

    /// Queues a transport error.
    pub fn queue_error(&self, error: HttpError) {
        self.state.borrow_mut().responses.push_back(Err(error));
    }

    /// Returns a list of all requests recorded so far.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.state.borrow().requests.clone()
    }

    /// Returns the most recent request sent, if any.
    pub fn last_request(&self) -> Option<HttpRequest> {
        self.state.borrow().requests.last().cloned()
    }

    /// Returns the number of responses remaining in the queue.
    pub fn remaining_responses(&self) -> usize {
        self.state.borrow().responses.len()
    }
}

impl HttpClient for FakeHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut state = self.state.borrow_mut();
        state.requests.push(request);
        match state.responses.pop_front() {
            Some(res) => res,
            None => Err(HttpError::Other(
                "no queued response available in FakeHttpClient".to_string(),
            )),
        }
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
    fn fake_client_records_requests_and_replays_responses() {
        let client = FakeHttpClient::new();
        client.queue_response(HttpResponse::new(200, b"ok"));
        client.queue_error(HttpError::Network("connection refused".to_string()));

        let req1 = HttpRequest::post("https://api.github.com/graphql")
            .with_header("authorization", "bearer test")
            .with_body(b"query");
        let resp1 = block_on(client.send(req1.clone())).unwrap();
        assert_eq!(resp1.status, 200);
        assert_eq!(resp1.body, b"ok");

        let req2 = HttpRequest::new(HttpMethod::Get, "https://api.github.com/user");
        let err2 = block_on(client.send(req2.clone())).unwrap_err();
        assert!(matches!(err2, HttpError::Network(_)));

        assert_eq!(client.requests().len(), 2);
        assert_eq!(client.last_request().unwrap().url, "https://api.github.com/user");
        assert_eq!(client.remaining_responses(), 0);
    }
}
