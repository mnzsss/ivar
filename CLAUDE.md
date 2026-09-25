# ivar

Orchestration CLI and hall runtime for multi-repository development. Local-only Rust binary published on crates.io.

## Architecture & Codebase Map

- `src/action/`: Core workflows and domain logic (`feature`, `session`, `graph`, `hall`, `repo`, `skill`, `sync`).
- `src/cli/`: CLI argument parsing via `clap` commands and options.
- `src/domain/`: Pure domain types, states, and business invariants.
- `src/git/`: Git operations layer (combining `git2` and shell CLI invocation).
- `src/store/`: Storage layout, manifest, SQLite graph database, and versioned JSON stores.
- `src/infra/`: Cross-cutting helpers (AST parsing, text classification, hashing).
- `tests/`: Comprehensive unit, integration, and e2e simulation suites.

## Engineering Norms & Checks

- Format: `cargo fmt --all --check`
- Clippy: `cargo clippy --all-targets --all-features -- -D warnings`
- Test suite: `cargo test --all-features`
- Graph simulation: `cargo test --profile e2e --test graph --all-features -- --ignored simulation`
- Zero compiler warnings tolerated (`RUSTFLAGS="-D warnings"`).

## Release & Changelog Policy

- **Never update `CHANGELOG.md` manually.**
- All releases and changelog entries are managed automatically by `release-plz` in GitHub Actions (`.github/workflows/release-plz.yml`).
- Use Conventional Commits (`feat(...)`, `fix(...)`, `chore(...)`) in commit messages so `release-plz` accurately determines semantic version bumps and generates changelog entries upon merge to `main`.
