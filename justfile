# tomet-cms development tasks.

default:
    @just --list

#[ Build ]

build:
    cargo build --workspace

release:
    cargo build --workspace --release

#[ Test & lint ]

test:
    cargo test --workspace

check:
    cargo check --workspace --all-targets

check-wasm:
    cargo check -p tomet-cms --target wasm32-unknown-unknown

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    nix fmt

#[ Housekeeping ]

clean:
    cargo clean
    rm -rf dist result

#[ Documentation ]

# Generate and update all derived documentation across the workspace.
docs:
    tomet export .
    tomet format -i .

# Check that all documents parse, format cleanly, and match export targets.
docs-check:
    tomet check .
    tomet format --check .
    tomet export --check .
    twrit check .
    catasto check .
