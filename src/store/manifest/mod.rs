//! `ivar.json` — the hall's identity, committed and team-shared.
//!
//! This is the file a teammate reviews in a pull request, and the file
//! `git pull && ivar sync` acts on. It is read far more often than written, and it
//! is the one file that never migrates itself (see [`super::versioned`],
//! [`Policy::Committed`](super::versioned)).
//!
//! # The schema, v1
//!
//! ```json
//! {
//!   "version": 1,
//!   "name": "acme",
//!   "providers": { "available": ["claude-code", "opencode"], "default": "claude-code" },
//!   "repos": [
//!     { "name": "api", "url": "git@github.com:acme/api.git", "default_branch": "main" }
//!   ],
//!   "skills": { "targets": { "claude": true, "opencode": true } }
//! }
//! ```
//!
//! `skills` is optional — a hall without a shared skill home simply omits it.
//!
//! # Why v1 and not v2
//!
//! The predecessor's `hall.json` is at `version: 2`, and v2 exists **only** to
//! carry server identity: `hallId`, `workspaceId`, `canonicalRevision`, and the
//! Workspace binding inside `skills`. `ivar` is local-only — there is no server to
//! be the source of truth, so those four fields do not exist here.
//!
//! Numbering this v2 would inherit a lineage no user outside the private monorepo
//! ever had, and would imply a v1 that was never public. It is a new file, with a
//! new name, in a new implementation: **v1**.
//!
//! The consequence is that there is no v0 → v1 migration to write. The chain
//! starts and ends at 1. It still must start at 0 structurally, so unversioned data
//! is rejected rather than silently adopted — a file with no `version` field is not
//! an `ivar.json`.
//!
//! # Contract
//!
//! - `Manifest` — `Deserialize` + `Serialize`, with
//!   `#[serde(deny_unknown_fields)]` on **every** struct here. A typo or a stale
//!   key is a hard parse error naming the key, not silence. This is a config a
//!   human hand-edits; silence is how a team ends up with a setting that does
//!   nothing.
//! - Fields typed with the newtypes from [`crate::domain::name`], so `../` cannot
//!   arrive through a hand-edited file. Remember that deriving `Deserialize` on
//!   the newtype must route through its validator.
//! - `read(&Layout)` — absent is `Ok(None)`; present-but-unparseable is a hard
//!   error. Discriminate on `NotFound` specifically.
//! - `write(&Layout, &Manifest)` — canonical JSON, atomic, and subject to the
//!   committed-file policy.
//! - Invariants validated on read, not just on write, because the file is
//!   hand-edited: `providers.default` must appear in `providers.available`;
//!   `available` must be non-empty; repo names must be unique; a repo's `url`
//!   must be non-blank.
//!
//! # Reference
//!
//! `packages/bifrost/src/manifest/schema.ts` and `manifest/index.ts` in the private
//! monorepo, read as a source of field semantics — **not** as a shape to copy. The
//! server fields and the `LEGACY_MANIFEST_FILE` fallback are both deliberately
//! absent here.
//!
//! # Invariant enforcement: explicit `validate`, not a validating `Deserialize`
//!
//! The four value invariants (`providers.default` in `providers.available`;
//! `providers.available` non-empty; unique repo names; non-blank repo `url`)
//! are checked by
//! [`Manifest::validate`], called explicitly by both [`Manifest::read`] and
//! [`Manifest::write`] — not baked into a hand-rolled `Deserialize` the way
//! [`crate::domain::name`]'s newtypes route through their validators.
//!
//! That is a deliberate departure from the newtype pattern, not an oversight.
//! Routing invariant checks through `Deserialize` would fold a violation into
//! `serde`'s error type before it ever reaches this module's [`Error`] — every
//! caller would see is a generic `store.schema_mismatch` with the detail buried
//! in a formatted string, not the specific variant, `code`, and fix action this
//! module's contract promises for each violation. So field-*shape* validation
//! (a `RepoName` cannot be `../etc`, a `Provider` cannot be an unknown id) stays
//! in `Deserialize`, via the newtypes; field-*relationship* validation runs after,
//! as an explicit step with full access to this module's own error type.
//!
//! [`Manifest::new`] runs the same check, so a `Manifest` built programmatically
//! (never touching JSON) is validated by construction. The one gap: nothing stops
//! `serde_json::from_value::<Manifest>` called directly (bypassing `Manifest::read`)
//! from producing a shape-valid, invariant-unchecked value — but nothing in this
//! crate does that, since the house rule is that all disk access goes through
//! [`versioned::Store`], and `Store::read` is exactly what `Manifest::read` wraps
//! and then validates.
//!
//! # Layout
//!
//! `model` owns the data types and the value invariants, `persistence` owns
//! read/write/plan/migrate, and `error` owns `Error` and its `Failure`
//! conversion. The facade below reexports the established public surface, so
//! callers keep importing `store::manifest::{Manifest, Repo, Providers, …}`.

mod error;
mod model;
mod persistence;
mod schema;

pub use error::Error;
pub use model::{Manifest, Providers, Repo, Skills, Targets};
pub use persistence::MigrationPlan;
pub use schema::generate;
