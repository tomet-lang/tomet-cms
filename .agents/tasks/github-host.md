# github-host: GitHub implementation of `Host`, with an injected HTTP client

Status: not started. Update this file after every step.
Depends on: save-hardening (the `commit_file` signature carries a message).

## Goal

A `Host` implementation for GitHub that works in the browser (wasm) and natively, without this
crate performing any HTTP itself. Everything is testable offline against a fake HTTP client.

## Background

- Decided 2026-10-29: GitHub only at first, but other hosts will follow soon, so nothing GitHub
  specific may leak into `tomet-cms` (core). The default branch only for now; the design must
  not preclude other branches later (the branch name is a constructor argument).
- HTTP is injected. The core and this crate never open a socket. In the browser the caller backs
  the client with `fetch`; natively with any HTTP library.
- Authentication: a personal access token, passed to the constructor. No OAuth in this task.
- Reference (read, do not copy blindly): Sveltia CMS,
  `src/lib/services/backends/git/github/{commits.js,graphql.js}` in
  https://github.com/sveltia/sveltia-cms. It uses GraphQL `createCommitOnBranch` with
  `expectedHeadOid`, base64 `contents`, and on failure re-fetches the branch head and compares it
  with the expected one instead of trusting the error message wording. Do the same.

## Design

- New crate `crates/github` (package `tomet-cms-github`). `tomet-cms` (core) gains no new
  dependency. The new crate may use `serde`, `serde_json`, `base64`. All must build for
  `wasm32-unknown-unknown`.
- An HTTP abstraction lives in the **new crate** (it is only needed by hosts that talk HTTP):
  a trait with one async method taking a plain request (method, url, headers, body bytes) and
  returning a plain response (status, headers, body bytes), and an error type for transport
  failures. Like `Host`, its futures are not `Send`; document why.
- `GitHubHost::new(http, owner, repo, branch, token)`. One endpoint: `POST
  https://api.github.com/graphql` for everything (head, read, commit). The API base URL is a
  field with that default so GitHub Enterprise can be pointed at later.
- `get_head`: query the branch ref's target commit oid.
- `read_file(path, rev)`: query the blob at `<rev>:<path>`; text only. Absent object -> `Ok(None)`.
  A binary or oversized blob is a typed error, not a lossy read.
- `commit_file(path, content, message, expected_head)`: `createCommitOnBranch` with
  `expectedHeadOid = expected_head`, one addition, base64 contents. On any failure, fetch the
  head again: if it differs from `expected_head` return `HostError::BranchMoved`, otherwise map
  the original error. Return the new commit oid as the revision.
- HTTP 401/403 and GraphQL errors map to typed variants the UI can act on (bad token vs no
  permission vs rate limit vs other). If `HostError` needs new variants for this, add them to the
  core in a separate, minimal change and say so in the Log.

## Steps

1. [ ] Create `crates/github`, add it to the workspace, add its wasm check to `just check-wasm`
   and CI.
2. [ ] HTTP trait + a fake client for tests (records requests, replays queued responses).
3. [ ] `get_head`, `read_file`, `commit_file` with request/response JSON handled by typed
   structs, not string concatenation of JSON. Unit tests per method using the fake client,
   including: file absent, blob binary, token rejected, rate limited, GraphQL `errors` array with
   HTTP 200.
4. [ ] The "branch moved" detection: tests where the commit fails and the follow-up head query
   shows a different head (-> `BranchMoved`) and the same head (-> original error mapped).
5. [ ] End-to-end save test: `tomet_cms::save` driven through `GitHubHost` with the fake client,
   covering a clean save and a save that retries once after `BranchMoved`.
6. [ ] An opt-in live test (`#[ignore]`, reads `GITHUB_TOKEN`, `GITHUB_REPO` from the
   environment) that does a real create, edit, concurrent-edit-conflict round trip on a scratch
   repository. It must never run by default or in CI. Do not run it yourself; leave it for the
   user.
7. [ ] Docs in `tmtroot/readme.tmt`: what the crate is, the HTTP trait contract, how to back it
   with `fetch`. Regenerate and check.

## Out of scope

OAuth, other hosts, other branches, caching, file deletion, multi-file commits, wasm-bindgen
bindings (including the `fetch`-backed client itself), any UI.

## Review checklist (for the reviewer)

- Core `tomet-cms` Cargo.toml is unchanged except for any minimal `HostError` addition.
- No HTTP library, no `std::net`, no runtime crate anywhere in the new crate.
- `cargo check --target wasm32-unknown-unknown` passes for both crates.
- Request bodies are built from typed structs; the token never appears in logs, `Debug` output or
  error text.
- "Branch moved" is decided by comparing heads, not by matching error message text.
- The live test is `#[ignore]` and was not run by the implementer.

## Log

(append dated entries here as steps complete)
