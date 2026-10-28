<!-- Generated from tmtroot/contributing.tmt. Edit that, then `tomet export .`. -->

# Contributing to tomet-cms

## Documentation Policy

- All Markdown files (`README.md`, `AGENTS.md`, `CONTRIBUTING.md`, etc.) are generated build artifacts.
Never edit `.md` files directly.
- Always edit the source `.tmt` files under `tmtroot/`.
- Run `just docs` (or `tomet export .`) to regenerate Markdown files, and verify with `just docs-check`.

## Commit Convention

All commits must follow the Conventional Commits format: `type(scope): description`.

### Types

- `feat`: New feature.
- `fix`: Bug fix.
- `docs`: Documentation changes (`.tmt` or generated docs).
- `refactor`: Code restructuring without changing behavior.
- `perf`: Performance improvement.
- `test`: Adding or correcting tests.
- `chore`: Build scripts, dependencies, tooling, repo maintenance.

### Scopes

Use the affected component as the scope:

- **CLI**: `cli` (`app`)
- **Infra / Meta**: `nix`, `deps`, `docs`

> [!tip]
> If a change spans multiple components or the whole workspace,
> omit the scope or use a broad category (e.g. `refactor: ...` or `chore: update dependencies`).

## Clean Commits Policy

Do not include AI session metadata or trailers in commit messages.
Keep commit logs clean and focused on technical changes.

