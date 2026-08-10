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

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domain::mcp::McpServerDef;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;
use crate::store::versioned::{self, Inspection, Policy, Store};

/// `ivar.json`'s schema version. See the "Why v1 and not v2" section above:
/// this is the first public version, with no v0 predecessor to migrate from.
const CURRENT_VERSION: u32 = 1;

/// What migrating `ivar.json` would do. Produced by [`Manifest::plan`], which
/// never touches the file.
///
/// Distinct from [`Inspection`] in one way that matters: `Inspection` reports
/// what the *version numbers* say, while this reports what would actually
/// happen — [`Self::Unreachable`] is the case where the file is older and there
/// is no chain to advance it, which `Inspection` calls `NeedsMigration` and
/// [`Manifest::migrate`] would refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum MigrationPlan {
    /// Already current. Nothing to do.
    Current { version: u32 },
    /// Older, with a migration chain that reaches the current version.
    Available { from: u32, to: u32 },
    /// Older, and no migration reaches it. For `ivar.json` this means a file
    /// with no `version` field, which is not an `ivar.json` at all.
    Unreachable { from: u32, to: u32 },
    /// Newer than this build understands. Refused, and never modified.
    TooNew { found: u32, highest: u32 },
}

/// The hall's identity, committed and team-shared. See the module doc comment
/// for the full JSON shape, the contract, and how the invariants are
/// enforced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    version: u32,
    name: HallName,
    providers: Providers,
    repos: Vec<Repo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skills: Option<Skills>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp: Option<Vec<McpServerDef>>,
}

impl Manifest {
    /// Build a validated `Manifest`. Refuses exactly what [`Self::validate`]
    /// refuses — see the module doc comment for why this is the one type-level
    /// guarantee this module makes: any `Manifest` built through this
    /// constructor already satisfies every invariant.
    pub fn new(
        name: HallName,
        providers: Providers,
        repos: Vec<Repo>,
        skills: Option<Skills>,
    ) -> Result<Self, Error> {
        let manifest = Self {
            version: CURRENT_VERSION,
            name,
            providers,
            repos,
            skills,
            mcp: None,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The schema version. Always [`CURRENT_VERSION`] for a value obtained
    /// through [`Self::new`] or [`Self::read`].
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The hall's name.
    #[must_use]
    pub fn name(&self) -> &HallName {
        &self.name
    }

    /// The hall's provider configuration.
    #[must_use]
    pub fn providers(&self) -> &Providers {
        &self.providers
    }

    /// The repos this hall knows about.
    #[must_use]
    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    /// The hall's shared skill home, if it has one.
    #[must_use]
    pub fn skills(&self) -> Option<&Skills> {
        self.skills.as_ref()
    }

    /// The hall-scoped MCP server definitions `ivar sync` materialises into
    /// each provider's config file at the hall root.
    ///
    /// Empty when the manifest carries none — the v1 common case. The materialiser
    /// still writes a valid (empty) config, so the file exists and the walk-up
    /// discovery contract holds.
    #[must_use]
    pub fn mcp_servers(&self) -> &[McpServerDef] {
        self.mcp.as_deref().unwrap_or_default()
    }

    /// Return a new `Manifest` carrying `servers` as its hall-scoped MCP
    /// definitions.
    ///
    /// Refuses (with [`Error::DuplicateMcpServerName`]) when two definitions
    /// share a `name` — duplicate names would silently collapse into one key
    /// in the generated config. An empty list is stored as *absent*, so a hall
    /// with no MCP servers round-trips byte-identical to one that never had
    /// the key.
    pub fn with_mcp_servers(&self, servers: Vec<McpServerDef>) -> Result<Self, Error> {
        let mut manifest = Self::new(
            self.name.clone(),
            self.providers.clone(),
            self.repos.clone(),
            self.skills.clone(),
        )?;
        manifest.mcp = if servers.is_empty() {
            None
        } else {
            Some(servers)
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Return a new `Manifest` with `repo` appended to `repos`.
    ///
    /// Returns [`Error::DuplicateRepoName`] if a repo with `repo.name()`
    /// already appears. The original is untouched — `ivar.json` is rewritten
    /// from the returned value, never mutated in place.
    pub fn with_repo_added(&self, repo: Repo) -> Result<Self, Error> {
        if self.repos.iter().any(|existing| existing.name == repo.name) {
            return Err(Error::DuplicateRepoName {
                name: repo.name().clone(),
            });
        }
        let mut repos = self.repos.clone();
        repos.push(repo);
        Self::new(
            self.name.clone(),
            self.providers.clone(),
            repos,
            self.skills.clone(),
        )
    }

    /// Return a new `Manifest` without the repo named `name`.
    ///
    /// Returns [`Error::RepoNotFound`] if no repo in `self.repos` carries
    /// that name. Removing never touches the filesystem — the repo's bare
    /// clone and worktrees stay until `ivar cleanup` (slice 8) is told to
    /// remove them.
    pub fn with_repo_removed(&self, name: &RepoName) -> Result<Self, Error> {
        let repos: Vec<Repo> = self
            .repos
            .iter()
            .filter(|repo| repo.name != *name)
            .cloned()
            .collect();
        if repos.len() == self.repos.len() {
            return Err(Error::RepoNotFound { name: name.clone() });
        }
        Self::new(
            self.name.clone(),
            self.providers.clone(),
            repos,
            self.skills.clone(),
        )
    }

    /// Read `ivar.json` from `layout`'s hall root.
    ///
    /// `Ok(None)` if the file is absent. A present-but-unparseable file, an
    /// unknown key, a schema version newer than this binary understands, or a
    /// violated invariant are all hard errors — see the module doc comment.
    ///
    /// A missing (or non-numeric) `version` field is refused by
    /// [`versioned::Store`] itself, as [`versioned::Error::NoMigrationPath`]:
    /// `ivar.json`'s chain is empty (see "Why v1 and not v2" above), so there
    /// is no v0 to migrate from and a file detected at v0 is not an
    /// `ivar.json`. This module only renames that refusal into its own terms.
    pub fn read(layout: &Layout) -> Result<Option<Self>, Error> {
        let manifest = match Self::open(layout).read() {
            Ok(manifest) => manifest,
            // The store refuses data it has no migration path to. For
            // `ivar.json` — chain empty, first public version 1 — that means
            // exactly one thing, and this module can say it in the user's
            // terms rather than the store's.
            Err(versioned::Error::NoMigrationPath { path, .. }) => {
                return Err(Error::MissingVersion { path });
            }
            Err(other) => return Err(Error::Store(other)),
        };

        let Some(manifest) = manifest else {
            return Ok(None);
        };
        manifest.validate()?;
        Ok(Some(manifest))
    }

    /// Write `manifest` to `layout`'s hall root: canonical JSON, atomic, and
    /// subject to [`Policy::Committed`] (a `write` while the on-disk file is
    /// older than [`CURRENT_VERSION`] refuses, directing the caller at
    /// `ivar migrate` — see [`versioned`]).
    pub fn write(layout: &Layout, manifest: &Self) -> Result<(), Error> {
        manifest.validate()?;
        Self::open(layout).write(manifest).map_err(Error::Store)
    }

    /// What an explicit migrate would do to `ivar.json`, without touching it.
    ///
    /// `Ok(None)` if there is no `ivar.json` at all. Never fails on a file this
    /// binary cannot open — reporting safely on such a file is the point, and
    /// is what lets `ivar migrate` describe a too-new hall instead of refusing
    /// to speak about it.
    pub fn plan(layout: &Layout) -> Result<Option<MigrationPlan>, Error> {
        let store = Self::open(layout);
        let Some(inspection) = store.inspect().map_err(Error::Store)? else {
            return Ok(None);
        };
        Ok(Some(match inspection {
            Inspection::Current => MigrationPlan::Current {
                version: CURRENT_VERSION,
            },
            Inspection::TooNew { detected, current } => MigrationPlan::TooNew {
                found: detected,
                highest: current,
            },
            Inspection::NeedsMigration { detected, current } => {
                // `NeedsMigration` says "older", not "reachable". Asking the
                // store settles which, so the preview cannot promise a
                // migration that the migrate itself would refuse.
                if store.has_migration_path(detected) {
                    MigrationPlan::Available {
                        from: detected,
                        to: current,
                    }
                } else {
                    MigrationPlan::Unreachable {
                        from: detected,
                        to: current,
                    }
                }
            }
        }))
    }

    /// Advance `ivar.json`'s on-disk schema version, writing the migrated form.
    ///
    /// This is the only way a committed file's version ever moves — see
    /// [`Policy::Committed`]. Callers are expected to have shown
    /// [`Self::plan`] and got a human's answer first; nothing here asks.
    ///
    /// `Ok(None)` if there is no `ivar.json`.
    pub fn migrate(layout: &Layout) -> Result<Option<Self>, Error> {
        let migrated = match Self::open(layout).migrate() {
            Ok(migrated) => migrated,
            // Same renaming `read` does: for this file, "no migration path"
            // means "no version field", and that is not an ivar.json.
            Err(versioned::Error::NoMigrationPath { path, .. }) => {
                return Err(Error::MissingVersion { path });
            }
            Err(other) => return Err(Error::Store(other)),
        };
        let Some(manifest) = migrated else {
            return Ok(None);
        };
        manifest.validate()?;
        Ok(Some(manifest))
    }

    /// The [`Store`] this module reads and writes `ivar.json` through. Built
    /// fresh on every call — cheap, and keeps this module stateless.
    fn open(layout: &Layout) -> Store<Self> {
        Store::new(
            layout.manifest(),
            Vec::new(),
            CURRENT_VERSION,
            Policy::Committed,
        )
    }

    /// The value invariants named in the module doc comment. See
    /// the "Invariant enforcement" section above for why this is an explicit
    /// step rather than folded into `Deserialize`.
    fn validate(&self) -> Result<(), Error> {
        if self.providers.available.is_empty() {
            return Err(Error::NoAvailableProviders);
        }
        if !self.providers.available.contains(&self.providers.default) {
            return Err(Error::DefaultProviderNotAvailable {
                default: self.providers.default,
                available: self.providers.available.clone(),
            });
        }

        let mut seen: HashSet<&RepoName> = HashSet::new();
        for repo in &self.repos {
            if !seen.insert(&repo.name) {
                return Err(Error::DuplicateRepoName {
                    name: repo.name.clone(),
                });
            }
            if repo.url.trim().is_empty() {
                return Err(Error::EmptyRepoUrl {
                    name: repo.name.clone(),
                });
            }
        }

        if let Some(servers) = &self.mcp {
            let mut seen: HashSet<&str> = HashSet::new();
            for server in servers {
                if !seen.insert(server.name.as_str()) {
                    return Err(Error::DuplicateMcpServerName {
                        name: server.name.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// A hall's provider configuration: which harnesses it knows about, and which
/// one `ivar session start` picks when none is named explicitly.
///
/// Unlike [`Manifest`], this type does not validate its own invariants at
/// construction — `providers.default` being a member of `providers.available`
/// is checked by [`Manifest::validate`], not here, because the check needs
/// nothing from `Providers` that `Manifest` cannot already see, and keeping
/// every invariant check in one place (rather than splitting it between this
/// type and `Manifest`) is what keeps [`Error`]'s variants exhaustive and easy
/// to audit against the module doc comment's list of three.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Providers {
    available: Vec<Provider>,
    default: Provider,
}

impl Providers {
    /// Build a `Providers` value. Not validated here — see the type doc
    /// comment for why; [`Manifest::new`] validates the whole `Manifest` this
    /// ends up inside.
    #[must_use]
    pub fn new(available: Vec<Provider>, default: Provider) -> Self {
        Self { available, default }
    }

    /// Every provider this hall knows about.
    #[must_use]
    pub fn available(&self) -> &[Provider] {
        &self.available
    }

    /// The provider `ivar session start` picks when none is named explicitly.
    #[must_use]
    pub fn default_provider(&self) -> Provider {
        self.default
    }
}

/// One repo a hall knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    name: RepoName,
    url: String,
    default_branch: BranchName,
}

impl Repo {
    /// Build a `Repo`. `url` stays a plain `String` rather than one of
    /// `domain::name`'s newtypes: it is a git remote, never joined onto a path,
    /// so none of the path-safety rules apply to it.
    ///
    /// It must still be non-empty, and that is checked by
    /// [`Manifest::validate`] alongside the other invariants rather than here —
    /// so it is enforced on **read** too, which is the case that matters, since
    /// this file is hand-edited. An empty `url` is the difference between a
    /// `Failure` naming the offending repo and a bare `git clone` error the
    /// first time someone runs `ivar sync`.
    #[must_use]
    pub fn new(name: RepoName, url: impl Into<String>, default_branch: BranchName) -> Self {
        Self {
            name,
            url: url.into(),
            default_branch,
        }
    }

    /// This repo's name. Unique among a manifest's repos — see
    /// [`Manifest::validate`].
    #[must_use]
    pub fn name(&self) -> &RepoName {
        &self.name
    }

    /// The git remote URL this repo clones from.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The branch a fresh worktree of this repo defaults to.
    #[must_use]
    pub fn default_branch(&self) -> &BranchName {
        &self.default_branch
    }
}

/// A hall's shared skill home: which harnesses skills materialise for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skills {
    targets: Targets,
}

impl Skills {
    /// Build a `Skills` value.
    #[must_use]
    pub fn new(targets: Targets) -> Self {
        Self { targets }
    }

    /// Which harnesses this hall's shared skills materialise for.
    #[must_use]
    pub fn targets(&self) -> &Targets {
        &self.targets
    }
}

/// Which harnesses a hall's shared skills materialise for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    claude: bool,
    opencode: bool,
}

impl Targets {
    /// Build a `Targets` value.
    #[must_use]
    pub fn new(claude: bool, opencode: bool) -> Self {
        Self { claude, opencode }
    }

    /// Whether skills materialise at `.claude/skills/`.
    #[must_use]
    pub fn claude(&self) -> bool {
        self.claude
    }

    /// Whether skills materialise at `.opencode/skills/`.
    #[must_use]
    pub fn opencode(&self) -> bool {
        self.opencode
    }
}

/// The comma-joined provider ids in `providers`, for error messages.
fn provider_ids(providers: &[Provider]) -> String {
    providers
        .iter()
        .map(Provider::id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Everything that can go wrong reading or writing `ivar.json`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Something failed at the [`Store`] layer: I/O, invalid JSON, an unknown
    /// key, a schema version newer than this binary understands, or the
    /// committed-file write policy. Delegates its `Failure` conversion
    /// entirely to the wrapped error, which already has its own code and fix
    /// action.
    #[error(transparent)]
    Store(#[from] versioned::Error),

    /// The file has no `version` field, or one that is not a JSON number. See
    /// the "Why v1 and not v2" section of the module doc comment: `ivar.json`
    /// has no v0, so unversioned data is not an `ivar.json` at all, not a file
    /// silently adopted as one.
    #[error("{path}: has no `version` field (or a non-numeric one); this is not an ivar.json")]
    MissingVersion { path: camino::Utf8PathBuf },

    /// `providers.default` does not appear in `providers.available`.
    #[error("default provider `{default}` is not in `providers.available`")]
    DefaultProviderNotAvailable {
        default: Provider,
        available: Vec<Provider>,
    },

    /// `providers.available` is empty.
    #[error("`providers.available` must not be empty")]
    NoAvailableProviders,

    /// Two repos share the same `name`.
    #[error("repo name `{name}` is used by more than one repo")]
    DuplicateRepoName { name: RepoName },

    /// A repo's `url` is empty or blank.
    ///
    /// Not a path-safety concern — a remote URL is never joined onto disk — but
    /// it is the difference between a `Failure` that names the offending repo
    /// and a raw `git clone` error the first time someone runs `ivar sync`.
    #[error("repo `{name}` has an empty `url`")]
    EmptyRepoUrl { name: RepoName },

    /// A repo named `name` is not in `self.repos` — `ivar repo remove` was
    /// asked to remove something the hall does not know about.
    #[error("repo `{name}` is not in ivar.json")]
    RepoNotFound { name: RepoName },

    /// Two MCP server definitions in `mcp` share the same `name`.
    #[error("MCP server name `{name}` is used by more than one definition")]
    DuplicateMcpServerName { name: String },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        // Re-typing it per arm is how the two drift — they already had.
        let what = error.to_string();

        match error {
            Error::Store(source) => source.into(),
            Error::MissingVersion { .. } => Failure::blocked("manifest.missing_version", what)
            .expected("a `version` field naming the schema version")
            .actual("no `version` field, or a non-numeric one")
            .fix(FixAction::safe(
                "manifest.add_version_field",
                "Add a `\"version\": 1` field to the manifest — every ivar.json has one.",
            )),
            Error::DefaultProviderNotAvailable { default, available } => {
                let ids = provider_ids(&available);
                Failure::blocked("manifest.default_provider_not_available", what)
                .expected(format!("`providers.default` to be one of: {ids}"))
                .actual(format!("`providers.default` is `{default}`, not in [{ids}]"))
                .fix(FixAction::safe(
                    "manifest.fix_default_provider",
                    format!(
                        "Add `{default}` to `providers.available`, or change `providers.default` to one of: {ids}."
                    ),
                ))
            }
            Error::NoAvailableProviders => {
                Failure::blocked("manifest.no_available_providers", what)
            }
            .expected("at least one provider id in `providers.available`")
            .actual("an empty `providers.available` list")
            .fix(FixAction::safe(
                "manifest.add_available_provider",
                "Add at least one provider id (`claude-code` or `opencode`) to `providers.available`.",
            )),
            Error::DuplicateRepoName { name } => Failure::blocked("manifest.duplicate_repo_name", what)
            .expected("every entry in `repos` to have a unique `name`")
            .actual(format!("`{name}` appears more than once in `repos`"))
            .fix(FixAction::safe(
                "manifest.rename_duplicate_repo",
                format!("Rename or remove the duplicate `{name}` entry in `repos` so the name appears once."),
            )),
            Error::EmptyRepoUrl { name } => Failure::blocked("manifest.empty_repo_url", what)
            .expected("a git remote URL")
            .actual("an empty string")
            .fix(FixAction::safe(
                "manifest.set_repo_url",
                format!("Set `url` on the `{name}` entry in `repos` to its git remote, or remove the entry."),
            )),
            Error::RepoNotFound { name } => Failure::blocked("manifest.repo_not_found", what)
            .expected(format!("`{name}` to be listed in `repos`"))
            .actual(format!("`{name}` does not appear in ivar.json"))
            .fix(FixAction::safe(
                "manifest.add_repo_first",
                format!("Add `{name}` with `ivar repo add`, or check the spelling."),
            )),
            Error::DuplicateMcpServerName { name } => {
                Failure::blocked("manifest.duplicate_mcp_server_name", what)
                .expected("every definition in `mcp` to have a unique `name`")
                .actual(format!("`{name}` appears more than once in `mcp`"))
                .fix(FixAction::safe(
                    "manifest.rename_duplicate_mcp_server",
                    format!("Give one of the duplicate `{name}` definitions a different name."),
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/manifest.rs"]
mod tests;
