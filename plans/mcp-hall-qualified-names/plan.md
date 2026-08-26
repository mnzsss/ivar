# Plan

Hall-qualified MCP server names at every provider boundary, and the file split
that makes `action/mcp/auth` reviewable while that change lands.

## Entities

**MCP server canonical name** — the `name` field of an `mcp` entry in
`ivar.json`. Unqualified and reusable across halls: `figma`, `linear`. It is the
identity the manifest validates for uniqueness, the identity `ivar mcp auth`
takes as its argument, and the identity user-facing output names.

**MCP server materialised name** — `<hall>-<server>`, derived from the hall's
own name and the canonical name: `acme-figma`. It is never stored in
`ivar.json`, never used to resolve a manifest entry, and exists only at the
provider boundary: the key in `.mcp.json` and `opencode.json`, the argument to
`claude mcp login` / `opencode mcp auth`, the key OpenCode writes into
`mcp-auth.json`, and the `<HALL>_<SERVER>` half of the OAuth secret variable
name.

Delta against `docs/glossary.md`'s **MCP** entry: the definitions are still
hall-scoped and secret-free, but the name written into a provider's file is now
derived rather than copied.

## Approach

Today `McpServerDef.name` carries two identities at once. `ivar.json` is a
committed, team-shared file, so a hall that wants a Figma server has to write
`figma-acme` into it by hand to keep the provider-side key unique and
recognisable — the hall's name leaks into the canonical config, and the same
`mcp` entry cannot be copied between halls.

Split the two identities. The manifest keeps `figma`. Every provider boundary
derives `<hall>-<server>` from the hall the manifest already names.

Rejected: **deriving the name inside `harness::config::mcp` from a `Layout`.**
`harness` may import `store`, but handing it a `Layout` to reach a hall name is
a wider dependency than the one fact it needs. It takes a `&HallName`.

Rejected: **pre-transforming `McpServerDef` values before materialisation.**
Cloning each definition with `name` replaced by the qualified form would make
`name` lie about canonical identity for the rest of its life, and nothing would
stop a later caller from persisting that clone back into `ivar.json` or matching
against it. The transformation stays a derivation, never a mutation.

Rejected: **a migration for existing manifests.** `ivar.json` is committed and
hand-edited; a hall carrying `figma-acme` today would derive
`acme-figma-acme` after this change. This is a deliberate breaking change:
the operator renames the entry to `figma` and re-runs `ivar sync`. A silent
prefix-stripping heuristic in `sync` would guess at intent on a file the user
owns.

Rejected: **a fallback that reads the old secret variable name.** One name, one
place it is built.

## Structure

`src/action/mcp/auth.rs` is 831 lines holding six concerns: the public input and
outcome types with their human rendering, manifest and provider resolution,
per-provider orchestration, Figma pre-registration and the secret handoff,
provider command construction and dispatch, and OpenCode token verification. The
naming change touches four of those six. Split first, following the
`action/sync/` and `action/feature/integrate/` precedent already in the tree.

```
src/action/mcp/
  mod.rs               unchanged, six lines: `pub mod auth;`
  auth/
    mod.rs             module doc (the three-step narrative — the map to the
                       children), AuthInput, Preregistration, ProviderRun,
                       AuthOutcome + WriteHuman, auth(), all_providers_report,
                       resolve_server, resolve_provider.
                       Declares `mod preregister; mod dispatch;` — private,
                       no re-export.
    preregister.rs     Preregistered, preregister_if_needed,
                       ensure_secret_env_set, secret_env_var,
                       print_secret_export, host_of.
                       Only Preregistered and preregister_if_needed are
                       `pub(super)`.
    dispatch.rs        Attempt, attempt, try_run_provider, run_provider,
                       verify_authenticated, auth_command, login_failed.
                       Attempt, try_run_provider, run_provider are
                       `pub(super)`.
```

The public path is unchanged byte-for-byte: `action::mcp::auth::{auth,
AuthInput, AuthOutcome}` resolves to the directory's `mod.rs` identically, so
`src/bin/ivar.rs:33` and `src/cli/root.rs:19` are untouched.

`src/harness/config/mcp.rs` stays one file. It is 242 lines under the 300-line
review trigger, holds one concern, and the naming change touches one line plus
one parameter. Splitting it would be folder symmetry for its own sake.

Test files mirror the split, per `ARCHITECTURE.md`'s linked-module rule. Each
production file `#[path]`-links its own, one `../` deeper than today:

```
tests/unit/action/mcp/auth.rs                 ← auth/mod.rs
tests/unit/action/mcp/auth/preregister.rs     ← auth/preregister.rs
tests/unit/action/mcp/auth/dispatch.rs        ← auth/dispatch.rs
```

## Changes

Two commits. The first is a pure move with no assertion edited, so the naming
diff that follows is readable rather than tangled with an 831-line rename.

### Commit 1 — split `action/mcp/auth` (no behaviour change)

1. Create `src/action/mcp/auth/mod.rs` from the current `auth.rs`, keeping the
   module doc, the four public types and `WriteHuman`, `auth`,
   `all_providers_report`, `resolve_server`, `resolve_provider`. Add
   `mod preregister;` and `mod dispatch;`.
2. Move `Preregistered`, `preregister_if_needed`, `ensure_secret_env_set`,
   `secret_env_var`, `print_secret_export`, `host_of` into
   `src/action/mcp/auth/preregister.rs`. Narrow visibility: `pub(super)` on
   `Preregistered` and `preregister_if_needed` only.
3. Move `Attempt`, `attempt`, `try_run_provider`, `run_provider`,
   `verify_authenticated`, `auth_command`, `login_failed` into
   `src/action/mcp/auth/dispatch.rs`. `pub(super)` on `Attempt`,
   `try_run_provider`, `run_provider` only.
4. Delete `src/action/mcp/auth.rs`.
5. Split `tests/unit/action/mcp/auth.rs` three ways per the Structure section.
   `declare_server` is needed by both the facade and preregister tests —
   duplicate the four-line helper rather than build a shared test module for
   it.
   - stays: `resolve_server_*` (3), `resolve_provider_*` (3),
     `auth_refuses_*` (3), `ok_run`, `failed_run`, and the
     `all_providers_report` cases.
   - to `preregister.rs`: `preregistration_not_needed_for_claude_code`,
     `preregistration_not_needed_without_a_url`,
     `preregistration_not_needed_for_a_host_off_the_allowlist`,
     `preregistration_skipped_when_the_manifest_already_carries_oauth`,
     `preregistration_skipped_path_fails_naming_the_variable_when_it_is_unset`,
     `secret_env_var_uppercases_and_folds_non_alphanumerics`,
     `print_secret_export_succeeds_against_real_stderr`, `host_of_*` (3).
   - to `dispatch.rs`: `auth_command_*` (4), `login_failed_*` (2).
6. Fix every `#[path]` link to `../../../../tests/unit/…`.
7. Update `ARCHITECTURE.md`'s module map and its test-layout list with the three
   new linked files.

### Commit 2 — hall-qualified materialised names

8. `src/domain/mcp.rs`: add `McpServerDef::materialised_name(&self, hall:
   &HallName) -> String`, returning `format!("{hall}-{}", self.name)`. This is
   the one place the qualified name is built. Document that `name` stays
   canonical and is never overwritten with the result.
9. `src/harness/config/mcp.rs`: `materialise_mcp` takes a `hall: &HallName`;
   `servers_doc` keys each entry by `server.materialised_name(hall)` instead of
   `server.name`.
10. `src/action/sync/providers.rs:149`: pass `manifest.name()`.
11. `src/action/mcp/auth/mod.rs`: keep resolving the manifest entry by canonical
    name — `resolve_server` is unchanged, and the CLI argument stays canonical
    (`ivar mcp auth figma`). Compute the materialised name once from
    `manifest.name()` and thread it into dispatch. `AuthOutcome.server` keeps
    the canonical name; only `ProviderRun.command` shows the qualified form,
    because it is the command that actually ran.
12. `src/action/mcp/auth/dispatch.rs`: `auth_command` and `verify_authenticated`
    both take the materialised name. These two must move together — OpenCode
    keys `mcp-auth.json` by whatever name it was handed, so a qualified login
    argument with a bare `has_tokens` lookup would make every successful
    OpenCode auth report `mcp.auth_not_verified`.
13. `src/action/mcp/auth/preregister.rs`: `secret_env_var` takes the
    materialised name, producing `IVAR_MCP_ACME_FIGMA_SECRET`. The
    re-materialisation after a successful registration passes `manifest.name()`
    into `materialise_mcp`. No fallback to any previous variable name.
14. Replace every hall-specific fixture name in tracked files with a neutral one
    (`acme`) and canonical server names (`figma`, `linear`):
    `tests/unit/action/mcp/auth*.rs`, `tests/unit/harness/config.rs`,
    `tests/unit/harness/opencode_auth.rs`, `tests/unit/domain/mcp.rs`,
    `tests/unit/cli/root.rs`, `docs/reference/on-disk-format.md`, and the
    `secret_env_var` doc comment. Git history is out of scope.
15. Docs: `docs/reference/on-disk-format.md`'s v3 example uses `"name":
    "figma"` and `IVAR_MCP_ACME_FIGMA_SECRET`, and states the breaking change —
    a hall whose entry is already hall-qualified must rename it by hand.
    `docs/glossary.md`'s **MCP** entry gains the canonical/materialised
    distinction. `docs/reference/commands.md` is generated; regenerate if the
    `ivar mcp auth` help text changes.

## Verification

- `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D
  warnings`, `cargo test --all-features` — green after each commit
  independently.
- Commit 1 changes no assertion. `cargo test --all-features` passes with the
  test bodies moved verbatim.
- `tests/architecture.rs` passes: the layering rule holds (`domain` gains no
  import; `harness` takes a `domain` type, which it already may), and the
  src/tests mirror covers all three new files with no orphan.
- New test: `materialised_name` returns `acme-figma` for hall `acme`, server
  `figma`, and leaves `McpServerDef.name` untouched.
- New test in `tests/unit/harness/config.rs`: the OpenCode and Claude Code
  documents key the server by `acme-figma` while the manifest entry is `figma`.
- New test in `tests/unit/action/mcp/auth/dispatch.rs`: `auth_command` renders
  `opencode mcp auth acme-figma` and `claude mcp login acme-figma`.
- Updated test in `tests/unit/action/mcp/auth/preregister.rs`:
  `secret_env_var("acme-figma")` is `IVAR_MCP_ACME_FIGMA_SECRET`.
- No tracked file outside `.git/` still carries the old hall-specific fixture
  name — grep for it once and confirm zero hits.
- `IVAR_UPDATE_DOCS=1 cargo test --test docs_reference` if the command surface
  moved; `cargo test --test docs_reference` must pass without it afterwards.
- Manual, once: a hall named `acme` declaring `figma` runs
  `ivar sync`, then `ivar mcp auth figma --all-providers`; `opencode.json` keys
  the server `acme-figma`, and the OpenCode leg reports authenticated rather
  than `mcp.auth_not_verified`.

## Norms

- Conventional Commits, signed off. Commit 1 is `refactor(mcp):`, commit 2 is
  `feat(mcp)!:` — the `!` is what `release-plz` reads for the breaking bump.
- `ARCHITECTURE.md`'s module map moves with the files, in the same commit.
- No test touches the network; the Figma registration boundary stays faked
  where it is faked today.
- `docs/reference/commands.md` is generated — never hand-edited.

## Safeguards

- **The secret must not widen its blast radius.** `secret_env_var`'s input
  changes; its contract does not. The value still reaches exactly stderr and the
  one dispatched child, and `Preregistered` stays non-`Serialize`.
- **Do not let the qualified name reach the manifest.** The write-back in
  `preregister_if_needed` rebuilds `McpServerDef` values — it must keep
  `existing.name` and only set `oauth`. A regression here would persist
  `acme-figma` into `ivar.json` and produce `acme-acme-figma` on the next sync.
- **`auth_command` and `verify_authenticated` are one change.** Splitting them
  across commits breaks every OpenCode authentication.
- **Manifest uniqueness is still checked on canonical names.**
  `Manifest::validate` is unchanged; two entries named `figma` remain a hard
  error, and the hall prefix is not a disambiguator.
- **Existing halls break loudly, not silently.** After this lands, a manifest
  still carrying `figma-acme` materialises `acme-figma-acme` and authenticates
  against a server key that does not match its old credentials. The
  on-disk-format note is the only warning the operator gets — it must be
  explicit.
