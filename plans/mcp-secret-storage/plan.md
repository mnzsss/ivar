# Durable local MCP client-secret storage

## Goal

Make `ivar mcp auth figma` a one-time user action for an OpenCode-backed hall. When Figma returns a pre-registered OAuth client secret, Ivar persists it in hall-local, gitignored storage and automatically supplies it to later OpenCode authentication and session processes.

Claude Code behavior must remain unchanged. No secret value may enter `ivar.json`, `opencode.json`, command output, serialized outcomes, logs, errors, or committed files.

## Current behavior and project conventions

### Findings

- `src/action/mcp/auth/preregister.rs` pre-registers OpenCode with Figma. A fresh registration returns `client_id` and `client_secret`; only the environment-variable name is persisted in the manifest.
- `src/action/mcp/auth/dispatch.rs` passes the fresh secret directly to the first `opencode mcp auth` child. This makes the initial authentication work, but the value disappears when that process ends.
- On subsequent `ivar mcp auth` runs, `preregister_if_needed` sees the manifest's existing OAuth registration and calls `ensure_secret_env_set`. It currently accepts only a variable inherited from the caller's environment.
- `src/harness/config/mcp.rs` materializes the OpenCode reference as `clientSecret: "{env:NAME}"`; Claude Code deliberately emits no OAuth block because it does not require Figma pre-registration.
- `src/store/layout.rs` centralizes every managed path. It already defines `.ivar/secrets/` as local, gitignored secret material, but explicitly describes that directory as hand-maintained and never written by Ivar.
- `Layout::gitignore_lines` ignores `.ivar/*`, so `.ivar/secrets/mcp.env` is ignored without another `.gitignore` rule.
- `src/action/session/start.rs` is the common process-construction seam for interactive provider sessions: it builds the harness command, applies `SessionEnv`, and then spawns it.
- Filesystem operations belong behind `src/infra/fs/`; subprocess environment mutations use `crate::infra::proc::Command`.

### Recommendation

Add one narrowly scoped store component for Ivar-managed MCP secrets, backed by:

```text
<hall>/.ivar/secrets/mcp.env
```

Do not introduce a generic dotenv subsystem or load arbitrary values into Ivar's own process environment. Parse only the constrained `KEY=VALUE` format Ivar writes, select only the exact secret variable named by an MCP server's `oauth.client_secret_env`, and apply selected values directly to child commands.

This is the smallest safe extension of the existing architecture. It keeps the feature under the MCP domain while respecting `Layout` and filesystem boundaries. It does, however, intentionally revise the existing invariant that Ivar never writes under `.ivar/secrets/`; the documentation and comments must identify `mcp.env` as the sole Ivar-managed exception while leaving all other files hand-maintained.

## Product behavior

### First OpenCode authentication

Given a Figma MCP definition without an existing OAuth registration:

1. `ivar mcp auth figma` pre-registers the client as it does today.
2. Before dispatching OpenCode, Ivar atomically stores the returned secret under the generated environment-variable name in `.ivar/secrets/mcp.env`.
3. Ivar persists only `client_id` and `client_secret_env` in `ivar.json` and materializes only the environment reference in `opencode.json`.
4. Ivar injects the in-memory secret into the same `opencode mcp auth` invocation, preserving the currently working first-run flow.
5. The user is no longer instructed to copy an `export` command. Human output may say that the credential was saved locally, but must not contain its value.

### Later authentication

Given an existing OAuth registration:

1. Resolve the required variable in this precedence order:
   - caller environment, to preserve explicit user override and current compatibility;
   - `.ivar/secrets/mcp.env`.
2. If found, inject it into `opencode mcp auth`.
3. If absent from both sources, return a targeted error naming the variable and the expected local file, without exposing any secret material.
4. Never re-register merely because the local secret is missing; that preserves the existing idempotence guarantee and avoids silently proliferating OAuth clients.

### OpenCode sessions

Before spawning an OpenCode session, load only the secret variables referenced by the hall's MCP definitions and inject values found in `.ivar/secrets/mcp.env` into the OpenCode child command. The caller's existing environment wins over the stored value.

Do not inject these secrets into:

- Claude Code sessions;
- setup scripts or session hooks;
- `ivar session env --json` or the OpenCode `shell.env` plugin;
- unrelated subprocesses;
- Ivar's own global process environment.

This narrow injection prevents the plugin's JSON output or shell tools from turning the secret into ordinary session metadata while still allowing OpenCode to resolve `{env:...}` when it reads `opencode.json`.

### Existing authenticated users

An existing hall may already contain `oauth.client_secret_env` but no local stored value. There is no safe way to reconstruct the original Figma secret from `client_id` alone.

For that case:

- continue accepting the caller environment;
- allow the next successful invocation carrying that environment value to backfill `.ivar/secrets/mcp.env`;
- otherwise report the missing credential with a remediation message;
- do not silently rotate or re-register the client in this slice.

## Proposed structure

### 1. Canonical path

Update `src/store/layout.rs`:

- add `Layout::mcp_secrets_env() -> Utf8PathBuf` returning `.ivar/secrets/mcp.env`;
- update the layout documentation to distinguish hand-maintained files in `secrets/` from this one Ivar-managed credential file;
- retain the existing `.ivar/*` ignore rule.

Do not construct this path in action or harness modules.

### 2. MCP secret store

Add a focused module such as `src/store/mcp_secrets.rs` and expose it from `src/store/mod.rs`. Its responsibility is only the local MCP credential map.

Suggested API shape:

```rust
pub struct McpSecrets { /* private ordered map */ }

impl McpSecrets {
    pub fn read(layout: &Layout) -> Result<Self, Failure>;
    pub fn get(&self, name: &str) -> Option<&str>;
    pub fn set_and_write(
        layout: &Layout,
        name: &str,
        value: &str,
    ) -> Result<Change, Failure>;
}
```

Exact names may follow nearby store conventions, but preserve these properties:

- no public API returns the full map for serialization or debugging;
- no secret-bearing type derives `Serialize`;
- debug output must be omitted or redacted;
- keys must satisfy the environment-variable grammar already produced by `secret_env_var` (`[A-Z_][A-Z0-9_]*` is sufficient for the managed file);
- values must round-trip safely without shell evaluation;
- duplicate keys, malformed records, and unsupported syntax fail closed with a path and line number, never with the raw line;
- comments and blank lines may be accepted if useful, but shell constructs (`export`, interpolation, command substitution, multiline shell syntax) are not executed;
- rewriting one key preserves all other valid key/value pairs semantically and renders deterministically;
- the write is atomic and ends with one newline.

Prefer a deliberately small parser over adding a general dotenv dependency. Define escaping explicitly. A simple, robust option is to write JSON-quoted values after `KEY=` and accept only that representation plus an unquoted restricted form; alternatively use a private JSON document despite the `.env` filename. Before implementation, choose one format and encode round-trip tests for spaces, quotes, backslashes, `=`, Unicode, and newlines. If compatibility with standard dotenv tools is a requirement, use double-quoted dotenv values with a tested escape implementation and document the accepted subset.

### 3. Secure filesystem write

Extend `src/infra/fs/` only as needed so the store can create and replace a sensitive file atomically with owner-only permissions on Unix.

Required behavior:

- ensure `.ivar/secrets/` exists;
- create temporary and final files with mode `0600`, avoiding a window where a newly created file inherits a permissive umask result;
- after replacement, verify or enforce `0600` on Unix;
- preserve cross-platform compilation and define the Windows behavior explicitly (best available user-local ACL semantics, with no Unix-mode assertion);
- surface permission and write failures without embedding file contents.

Do not implement secure writes directly in the action layer, because the project convention confines managed `std::fs` access to `infra/fs`.

### 4. Authentication secret resolution

Refactor `src/action/mcp/auth/preregister.rs` and `dispatch.rs` so secret acquisition has one explicit result consumed by command construction.

For an existing OAuth registration:

- replace `ensure_secret_env_set` with a resolver that checks the caller environment first and then the MCP secret store;
- return the resolved `(name, value)` only in a private, non-serializable type;
- if the caller environment supplied the value and the store lacks it, persist it to backfill existing halls;
- report `Preregistration::Skipped` as today;
- pass the resolved secret to `auth_command`, rather than relying on ambient inheritance.

For a fresh registration:

- persist the secret before launching OpenCode;
- if persistence fails, stop before OAuth dispatch so the command cannot appear successful while leaving a non-durable setup;
- retain the in-memory handoff to the immediate child;
- replace `print_secret_export` with a non-secret confirmation message or remove it;
- ensure `AuthOutcome`, `ProviderRun`, `Attempt`, command display strings, and `--json` remain secret-free.

Keep preregistration decisions in `preregister.rs`; keep provider command dispatch in `dispatch.rs`. Do not move this behavior into `harness/mod.rs`, whose responsibility is provider command shape rather than hall credential storage.

### 5. Session command injection

Add a small MCP-auth helper that derives a command environment delta from:

- the selected provider;
- `manifest.mcp` definitions carrying OAuth references;
- caller environment precedence;
- `.ivar/secrets/mcp.env`.

Call it in `src/action/session/start.rs` after `SessionEnv::apply` and before PTY or non-TTY spawn.

Behavior:

- return the command unchanged for Claude Code;
- for OpenCode, inspect only configured `client_secret_env` names;
- do not fail a session merely because an optional MCP secret is absent unless existing OpenCode behavior already makes that MCP mandatory; prefer leaving the variable absent and allowing unrelated work to start;
- malformed or insecure credential storage should produce a clear warning or hard failure based on project error conventions. Recommended: fail closed for an unreadable/malformed file because silently ignoring a credential file can trigger confusing OAuth behavior, while a simply absent file remains valid;
- apply the stored value only when the caller environment does not already define the variable.

Do not add secrets to `SessionEnv`: that type has JSON-facing session metadata semantics and is also used by `ivar session env`. Keep credential injection as a separate step on the child command.

### 6. Documentation and architectural contracts

Update concrete references that currently state Ivar never stores secrets:

- `src/domain/mcp.rs` — clarify that canonical config stores references only, while a local credential store may hold the referenced value;
- `src/store/layout.rs` — document `mcp.env` and its ownership;
- `docs/glossary.md` and `docs/reference/on-disk-format.md` — identify the local file, permission posture, and non-committed lifecycle;
- relevant `src/action/mcp/auth` module documentation — replace the in-memory-only handoff contract with the durable local handoff;
- `ARCHITECTURE.md` — list the MCP secret store under `store/` if its module inventory is exhaustive;
- user-facing MCP auth documentation or command help — state that one successful auth persists the local credential and explain how to remove it.

Avoid documenting raw secret examples. Use placeholders.

## Test plan

Follow the repository's existing unit-test colocation pattern and add the smallest integration coverage necessary.

### Store tests

Add `tests/unit/store/mcp_secrets.rs` covering:

1. missing file reads as an empty store;
2. first write creates `.ivar/secrets/mcp.env`;
3. resulting file is `0600` on Unix;
4. setting a second key preserves the first;
5. updating a key replaces only that value;
6. deterministic ordering and trailing newline;
7. round trips spaces, quotes, backslashes, `=`, Unicode, and newline according to the chosen format;
8. malformed key/value syntax fails without including the raw secret-bearing line in the error;
9. duplicate-key policy is deterministic and tested;
10. no `Debug`/serialization path exposes values (prefer API design that makes this impossible rather than snapshotting a redaction).

### MCP auth tests

Extend `tests/unit/action/mcp/auth/preregister.rs` and `dispatch.rs`:

1. fresh Figma registration persists the returned secret before dispatch;
2. fresh secret is still injected into the same auth child;
3. existing registration resolves from caller environment;
4. caller environment overrides stored value;
5. caller environment backfills a missing stored value for an existing hall;
6. existing registration resolves from stored value when caller environment is absent;
7. missing value in both places returns the targeted error;
8. Claude and non-Figma servers never read or write the store;
9. persistence failure prevents auth command execution;
10. human and JSON outcomes contain no secret value;
11. command display contains the variable name at most, never its value.

Where the live Figma registration call currently resists deterministic testing, extract the narrow registration boundary or inject a test double rather than making networked tests.

### Session tests

Extend session-start command tests or add a focused helper test:

1. OpenCode receives configured stored MCP secret variables;
2. caller environment wins over stored values;
3. Claude Code receives none;
4. unrelated entries in `mcp.env` are not injected;
5. missing `mcp.env` does not block a normal session;
6. malformed credential storage follows the selected failure policy;
7. `ivar session env --json` does not include MCP secret variables or values.

### Security regression tests

Use a distinctive sentinel secret and assert it is absent from:

- serialized `AuthOutcome`;
- human output;
- error messages;
- displayed command strings;
- `ivar.json`;
- `opencode.json`;
- session environment JSON;
- any committed-path fixture.

### Verification commands

Run the repository's standard gates, at minimum:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Also run the existing architecture tests and the MSRV/cross-platform checks used by CI if they are not included by the commands above. Pay particular attention to Unix permission tests being correctly gated so Windows builds compile.

## Rollout and compatibility

- No manifest schema migration is needed: `client_secret_env` already exists and remains the canonical reference.
- No `opencode.json` format change is needed.
- Halls with no Figma/OpenCode OAuth configuration are unaffected.
- Existing users who exported the variable keep working; their explicit environment value takes precedence and can seed the new local file.
- Existing OpenCode tokens remain untouched. This feature does not write `mcp-auth.json`.
- Removing `.ivar/secrets/mcp.env` returns to the current external-environment behavior.

## Explicit non-goals

- A general-purpose `.env` loader for repositories, hooks, or agent shells.
- Persisting OAuth access or refresh tokens; OpenCode owns `mcp-auth.json`.
- Writing secrets into `ivar.json`, `.mcp.json`, or `opencode.json`.
- Changing Claude Code's Figma OAuth path.
- Automatically rotating or re-registering a client when a stored secret is missing.
- Synchronizing secrets across machines or hall clones.
- OS keychain integration in this slice.
- Exposing secret management through `ivar session env` or the OpenCode plugin.

## Implementation order

1. Add the `Layout::mcp_secrets_env` accessor and update layout ownership comments.
2. Add secure atomic sensitive-file support in `infra/fs`, with Unix mode tests.
3. Implement the focused MCP secret store and parser with round-trip and redaction tests.
4. Change Figma preregistration to persist fresh secrets and resolve existing ones from environment/store precedence.
5. Update auth dispatch to receive explicit private secret material and remove the printed export value.
6. Inject referenced stored secrets into OpenCode session child commands only.
7. Add migration/backfill behavior for existing halls whose variable is supplied externally.
8. Update user-facing and architectural documentation.
9. Run formatting, unit/integration tests, clippy, architecture checks, and platform-specific CI checks.

## Acceptance criteria

- A user can run `ivar mcp auth figma` once in an OpenCode hall and later start OpenCode sessions without manually exporting `IVAR_MCP_<HALL>_FIGMA_SECRET`.
- The persisted value exists only in `.ivar/secrets/mcp.env`, with owner-only Unix permissions and atomic updates.
- The value is supplied to immediate and future OpenCode auth/session child processes but not to Claude, hooks, session-env JSON, or unrelated subprocesses.
- Explicit caller environment values override stored values.
- Existing halls can backfill the store without re-registering their OAuth client.
- No secret value appears in committed config, output, logs, errors, serialization, or command display.
- Existing OpenCode token storage remains owned by OpenCode and is never modified by Ivar.
- All repository quality gates pass on supported platforms.
