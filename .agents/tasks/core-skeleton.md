# core-skeleton: workspace split, wasm gate, host trait, in-memory save flow

Status: complete.

## Goal

Establish the shape of `tomet-cms` as a UI-less, I/O-free library that can be compiled to
`wasm32-unknown-unknown`, with a save flow that is fully testable without a network.
Nothing in this task talks to GitHub.

## Background (decided with the user, 2026-10-29)

- `tomet-cms` is the engine behind `tomet-wiki`: a browser-side (wasm) library that edits a tomet
  vault stored in a Git host, using the host's HTTP API. There is no local clone and no server.
- Git never sees a conflict. Merging is done by `tomet::merge::merge` (AST-level three-way merge,
  pure Rust). A merge that cannot narrow a disagreement writes an `@conflict(a: .., b: ..)`
  element into the document; the result is committed as an ordinary file. Resolving a conflict
  means choosing one side and stripping the element.
- Save flow: the caller edited from a `base` revision; at save time fetch the current head
  (`remote`), merge `base`/`local`/`remote`, and commit the result **on top of that head**. If
  the head moved between fetch and commit, fetch again and retry.
- No UI framework, no CLI. The Svelte UI lives in `tomet-wiki`, not here.
- Not a goal now: renaming/moving pages, multi-file saves, attachments, OAuth, branches other
  than the default one, hosts other than GitHub (the trait must not preclude them).

## Decisions already made

- Host trait is `async`. Do not add an async runtime dependency; the trait returns futures and the
  caller drives them. Do not require `Send` on those futures (wasm is single-threaded); keep that
  a deliberate, documented choice.
- HTTP is not performed by this crate. A host implementation receives its HTTP client from the
  caller. (The GitHub implementation is a later task.)
- One save = one file = one commit. Return a commit-result type so multi-file saves can be added
  later without breaking callers.
- Modify/delete conflict (one side edited a file, the other deleted it): follow git. Git reports
  it as a conflict and leaves the edited version in the working tree. Equivalent here: keep the
  edited content, and report the condition in the save result as a typed warning. Do not write
  any marker into the document (a deleted side has no content to put in `@conflict`).
  Provisional; do not build policy configuration around it.
- Non-`.tmt` files: out of scope. Reject them with a typed error rather than choosing a policy.

## Steps

1. [x] **Remove the CLI.** Delete `app/` and the `clap`/`anyhow` workspace dependencies it needed.
   Update `tmtroot/readme.tmt` ("Usage", "Workspace Layout") and `tmtroot/agents.tmt` if they
   mention the CLI, then regenerate with `just docs` and verify with `just docs-check`.
   Never edit the generated `.md` files directly.
2. [x] **Create the `core` crate** (`crates/core`, package `tomet-cms`). Depend on `tomet` with
   only the features needed (`parser`, `printer`, `merge`; verify the names in the tomet repo).
   Tomet crates are git dependencies pinned by `Cargo.lock`; do not use `[patch]` or path
   dependencies (see `AGENTS.md`).
3. [x] **wasm gate.** Add a `just` recipe and a CI step that runs
   `cargo check -p tomet-cms --target wasm32-unknown-unknown`. Add the target to the Nix dev
   shell. This must pass before any later step is considered done. If `tomet` or one of its
   dependencies fails to build for wasm, stop and report which crate and why; do not work around
   it by disabling features silently.
4. [x] **Define the host trait** (module `host`). Minimum surface:
   - read a file at a given revision (returns content and an opaque revision identifier);
   - get the current head revision identifier;
   - commit one file change on top of an expected head, failing with a distinct error if the head
     has moved ("branch moved"), distinguishable from other errors.
   Revision identifiers are opaque strings to the core. Define the error type with `thiserror`-style
   enums (no `anyhow` in the public API). Document each method's contract in doc comments.
5. [x] **In-memory host** (module `memory`, public, used by tests and by downstream crates'
   tests). It must be able to simulate: head advancing between fetch and commit, a file being
   added/modified/deleted by another party, and a commit being refused with "branch moved".
6. [x] **Save flow** (module `save`). Input: path, `base` revision id, `local` content. Steps:
   fetch head, read `remote` at head, read `base` at the base revision, parse the three, call
   `tomet::merge::merge`, print the result, commit on top of head, retry on "branch moved" with a
   bounded number of attempts. Cover with tests against the in-memory host:
   - no one else changed the file (fast path: remote == base);
   - other party changed a different node (merges silently, no `@conflict`);
   - other party changed the same node (result contains `@conflict`, still committed);
   - head moves during commit (retries, then succeeds);
   - head keeps moving (gives up with a typed error after the bound);
   - remote deleted the file while local edited it (edited content kept, warning in the result);
   - file created fresh (no base);
   - non-`.tmt` path (typed error).
7. [x] **Conflict API** (module `conflict`). Given document text: list the `@conflict` elements
   (position/address, the two sides); resolve one by choosing `a` or `b`, returning new text with
   the element stripped and the rest byte-identical. Tests include a document with several
   conflicts and a conflict nested in `{data}` (scalar sides) as well as in block content.
8. [x] **Docs.** Describe the crate in `tmtroot/readme.tmt` (what it is, what it is not, the save
   flow, the wasm constraint). Regenerate and check.

## Out of scope for this task (do not start)

GitHub host implementation, wasm-bindgen bindings, caching, OAuth, any HTTP code, any UI.

## Review checklist (for the reviewer)

- `cargo check --target wasm32-unknown-unknown` passes on a clean checkout.
- No `std::fs`, `std::net`, `std::time::SystemTime`, threads, or runtime crates in `core`.
- No `Send` bound on the host futures, and the doc comment says why.
- Public error types are enums; "branch moved" is its own variant.
- Tests exercise the in-memory host only; no test touches the network.
- Modify/delete behaviour is the one stated above, and is documented as provisional.
- No generated `.md` edited by hand; `just docs-check` passes.

## Log
- 2026-10-29: Step 1 completed: removed CLI `app/`, removed clap/anyhow dependencies, updated `tmtroot/readme.tmt` and `tmtroot/contributing.tmt`, ran `just docs` and verified `just docs-check`.
- 2026-10-29: Step 2 completed: created `crates/core` (`tomet-cms` package), depended on `tomet` (`parser`, `printer`, `merge`).
- 2026-10-29: Step 3 completed: added wasm32-unknown-unknown target to nix dev shell and enabled `getrandom` `js` feature for wasm32; added `check-wasm` recipe to `justfile` and `.github/workflows/ci.yml`. Verified `cargo check -p tomet-cms --target wasm32-unknown-unknown` succeeds.
- 2026-10-29: Step 4 completed: defined `Host` trait with non-Send async futures in `crates/core/src/host.rs`, typed enums for `HostError` with `BranchMoved`, `Revision`, `FileContent`, and `CommitResult`. Verified compiles under wasm.
- 2026-10-29: Step 5 completed: implemented `MemoryHost` in `crates/core/src/memory.rs` with simulation capabilities for head movements, remote edits/deletions, and branch-moved errors with tests. Verified all tests pass and passes wasm check.
- 2026-10-29: Step 6 completed: implemented `save` flow in `crates/core/src/save.rs` with three-way merge, fast path, modify/delete conflict warning, fresh file handling, and bounded retry on branch movement. Covered with 8 exhaustive tests against `MemoryHost`. Verified passes wasm check.
- 2026-10-29: Step 7 completed: implemented `conflict` module in `crates/core/src/conflict.rs` with `list_conflicts` and `resolve_conflict` using span-based text slicing for byte-identical preservation. Covered with comprehensive tests across block, inline, and data-nested conflicts. Verified passes wasm check.
- 2026-10-29: Step 8 completed: updated `tmtroot/readme.tmt` with crate architecture, save flow, and wasm constraints. Regenerated derived Markdown docs and verified `just docs-check` passes. Task complete.

