//! Reading and writing `ivar.json`: `Manifest::read`, `Manifest::write`,
//! `Manifest::plan`, and `Manifest::migrate`, all through the versioned
//! [`Store`] with the committed-file policy.
//!
//! The data these move around lives in [`model`](super::model); the errors
//! they can return live in [`error`](super::error). `MigrationPlan` — what an
//! explicit migrate would do, without touching the file — lives here because
//! it is produced by [`Manifest::plan`] and consumed by `ivar migrate`.

use serde::Serialize;

use crate::store::layout::Layout;
use crate::store::versioned::{self, Inspection, Migration, Policy, Store};

use super::error::Error;
use super::model::{CURRENT_VERSION, Manifest};

/// Migrate a manifest from v1 → v2.
///
/// v2 adds the hall integration defaults (`integration`, with the embedded
/// `local`/`squash` values) and each repo's ordered `checks` list (empty by
/// default). Both fields are `#[serde(default)]`, so a v1 file would
/// deserialise without this step's help; the step still fills the explicit
/// shapes so the persisted v2 is canonical, and so a teammate whose binary is
/// older refuses the migrated file loudly rather than silently reading past
/// the new keys.
fn v1_to_v2(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("manifest must be an object")?;
    root.entry("integration").or_insert_with(|| {
        serde_json::json!({
            "strategy": "squash",
            "via": "local"
        })
    });
    let repos = root
        .get_mut("repos")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("manifest is missing repos")?;
    for repo in repos {
        repo.as_object_mut()
            .ok_or("repo must be an object")?
            .entry("checks")
            .or_insert_with(|| serde_json::json!([]));
    }
    Ok(value)
}

/// Migrate a manifest from v2 → v3.
///
/// v3 adds `McpServerDef.oauth` — an optional pre-provisioned OAuth client
/// registration (`client_id`, `client_secret_env`) for an MCP server whose
/// host rejects a harness's own dynamic client registration. The field is
/// optional and `#[serde(skip_serializing_if = "Option::is_none")]`, so a v2
/// manifest — which never has an `oauth` entry carrying it — already
/// deserialises against the v3 shape without help. This step touches no
/// data; it exists so the chain stays contiguous and [`Manifest::migrate`]
/// has a registered step to run, which is what advances the version number
/// stamped on disk and lets `ivar migrate` describe the step honestly rather
/// than reporting nothing to do on a v2 file.
fn v2_to_v3(value: serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(value)
}

/// Migrate a manifest from v3 → v4.
///
/// v4 adds `McpOauth.token_url` and `McpOauth.resource` — optional discovered
/// endpoint metadata for OAuth-enabled MCP servers. The fields are optional and
/// `#[serde(skip_serializing_if = "Option::is_none")]`, so a v3 manifest already
/// deserialises against the v4 shape without help. This step touches no data;
/// it exists so the chain stays contiguous and [`Manifest::migrate`] has a
/// registered step to run.
fn v3_to_v4(value: serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(value)
}

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

impl Manifest {
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
    /// Validation gates the write. A file whose content the current binary
    /// refuses — a retired MCP transport, say — is left exactly as it was
    /// rather than stamped to the new version with the offending content
    /// intact: that combination is unreadable afterwards, and the failed
    /// migration would be indistinguishable from a completed one.
    ///
    /// `Ok(None)` if there is no `ivar.json`.
    pub fn migrate(layout: &Layout) -> Result<Option<Self>, Error> {
        // `read` migrates in memory and validates; under `Policy::Committed`
        // it leaves the file untouched, so nothing is written until the
        // migrated form is known to be valid.
        let Some(manifest) = Self::read(layout)? else {
            return Ok(None);
        };
        Self::open(layout)
            .migrate()
            .map_err(Error::Store)?
            .ok_or_else(|| Error::MissingVersion {
                path: layout.manifest(),
            })?;
        Ok(Some(manifest))
    }

    /// The [`Store`] this module reads and writes `ivar.json` through. Built
    /// fresh on every call — cheap, and keeps this module stateless.
    ///
    /// The migration chain begins at v1, the format's first public version.
    /// Through the store's baseline semantics, a v1 file migrates in memory on
    /// read (and, on an explicit migrate, on disk); a v0 (unversioned) file
    /// has no migration path and is refused — mapped to [`Error::MissingVersion`]
    /// by [`Self::read`].
    fn open(layout: &Layout) -> Store<Self> {
        Store::new(
            layout.manifest(),
            vec![
                Migration::new(1, 2, v1_to_v2),
                Migration::new(2, 3, v2_to_v3),
                Migration::new(3, 4, v3_to_v4),
            ],
            CURRENT_VERSION,
            Policy::Committed,
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/store/manifest/persistence.rs"]
mod tests;
