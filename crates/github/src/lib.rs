//! GitHub implementation of [`tomet_cms::Host`].
//!
//! Provides a [`Host`] implementation backed by GitHub's GraphQL API,
//! with an injected HTTP transport client suitable for browser wasm (`fetch`)
//! and native environments.

#![forbid(unsafe_code)]

pub mod host;
pub mod http;

pub use host::GitHubHost;
pub use http::{FakeHttpClient, HttpClient, HttpError, HttpMethod, HttpRequest, HttpResponse};
