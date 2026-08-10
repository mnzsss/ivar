//! The schema-version machine: detect, migrate in order, refuse the future.
//!
//! One implementation, **two policies**. That split is the whole point of this
//! module, and getting it wrong breaks other people's repositories.
//!
//! # The two policies
//!
//! | policy | files | behaviour |
//! |---|---|---|
//! | [`Policy::Local`] | `.ivar/state.json`, lockfiles | migrates on read and persists the migrated form, silently. Nobody sees it. |
//! | [`Policy::Committed`] | `ivar.json` | reads an older version fine, and **never writes the new format on its own**. Writing requires an explicit migrate command. |
//!
//! Why `Committed` exists: `ivar.json` is committed and team-shared. If upgrading
//! `ivar` silently rewrote it, that becomes a commit — and a teammate whose binary
//! is older then refuses that commit as "a version I do not understand". One
//! person's upgrade would break someone else's checkout. So migrating a committed
//! file is a **team event**, and a human triggers it.
//!
//! # The published promise
//!
//! Exactly one sentence: *there will never be a hall you cannot open.* Not "we do
//! not break the format" — at `0.x` we will. It is that every format change ships
//! its migration, **and the chain is never pruned**: it always starts at v0, so a
//! hall from any past version still opens.
//!
//! # Contract
//!
//! - [`detect_version`] — a missing `version` field, or one that is not a JSON
//!   number, means 0. Not an error; unversioned data predates versioning.
//! - A [`Store<T>`] built from: a path, an ordered migration chain
//!   ([`Migration`]), the current version, and a [`Policy`].
//! - [`Store::read`] — absent is `Ok(None)`. Data newer than `current` is a
//!   **hard refusal** that names the versions and tells the user to upgrade
//!   `ivar`, and does not modify the file. Under `Local`, a migrated read
//!   persists; under `Committed`, it does not.
//! - [`Store::write`] — validates, stamps `version: current`, writes atomically.
//!   Refuses (for both policies) if the file on disk is newer than `current`.
//!   Under `Committed`, additionally refuses when the file on disk is older
//!   than `current`, directing the caller to [`Store::migrate`].
//! - [`Store::inspect`] — what migration *would* happen, mutating nothing. Safe
//!   to call even on a file `read` would refuse, because it never asks the file
//!   to become a `T`. This is what `doctor` and the migrate command report from.
//! - [`Store::migrate`] — the explicit escape hatch. Reads, migrates in memory,
//!   and persists the result if that advanced the version — regardless of
//!   policy. `Local` never needs it (every read already does this); `Committed`
//!   is the reason it exists.
//!
//! Migrations run on [`serde_json::Value`], not `T` — a step from an old shape
//! generally does not parse as the *current* `T` yet, which is exactly why the
//! step exists. The store stamps `version: current` onto the fully-migrated
//! value before ever handing it to `T`'s `Deserialize`, so `T` can declare its
//! own `version: u32` field (as `ivar.json`'s schema does) without every
//! individual migration having to remember to set it.
//!
//! # Invariants the constructor asserts
//!
//! The chain starts at 0, is contiguous (no gaps, no overlaps), and its last
//! step lands exactly on `current` — when the chain is non-empty. An **empty**
//! chain at a nonzero `current` is legitimate and deliberately not rejected: it
//! is `ivar.json`'s own case (see `store::manifest`) — a file whose first public
//! version is 1, with no v0 predecessor to migrate from, so there is no v0 → v1
//! migration to write. A v0 (unversioned) file handed to a store like that is not
//! a versioned-store error either; nothing transforms it, and it is deserialized
//! as-is, so it fails or succeeds purely on whether its shape matches `T` — "a
//! caller may still reject it on schema grounds", not because this module says
//! so.
//!
//! These are programmer errors: a gap or a mismatched terminus is wrong the
//! moment the binary that shipped it was built, not something a user's hall can
//! ever trigger. So they are asserted at construction (`assert_eq!`, which
//! panics — not a `Result`), the same way a malformed regex or a bad format
//! string would be, rather than surfacing as a runtime `Failure` on someone's
//! machine years later.
//!
//! # Modelling the two policies: a runtime enum, not a type parameter
//!
//! [`Policy`] is a field on [`Store<T>`], not a type parameter (there is no
//! `Store<T, Local>` / `Store<T, Committed>`). The fact that decides how `write`
//! behaves — whether the version already on disk is older than `current` — is
//! only knowable by reading the file, so the branch it drives is inherently a
//! runtime check with a runtime `Failure`, never a compile-time one. A marker
//! type would buy nothing here: it cannot stop a caller from constructing the
//! wrong pairing of policy and file (that mistake lives in `store::manifest` /
//! `store::hall_state` wiring a `Layout` path to a `Store::new` call, not in the
//! type of the data), and every method's signature is identical across both
//! policies — only the body's behaviour differs. A generic parameter would add a
//! phantom type argument to every call site for zero additional safety.
//!
//! # Reference
//!
//! `packages/bifrost/src/state/versioned.ts` in the private monorepo is the
//! working TypeScript implementation, including the chain-validity assertions.
//! It has one policy, not two — the `Committed` half is new here, and is the
//! reason this module is a rewrite rather than a transliteration. Two smaller
//! divergences, both deliberate: the TypeScript keeps the raw JSON object
//! shape through `atomicWrite`'s `{...validated, version}` spread, where this
//! module holds everything as `serde_json::Value` end to end and lets `T`'s own
//! `Deserialize`/`Serialize` do the shape work; and `isUnknownFutureVersion` /
//! `UnknownVersionError` are folded into one `Error::TooNew`, checked at both
//! `read` and `write`, rather than only guarding the read path — a version
//! newer than the binary understands must never be overwritten either, and the
//! TypeScript predecessor had no `write` path of its own to need that.

use std::fmt;
use std::marker::PhantomData;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Failure, FixAction};
use crate::infra::json;

/// Which migration policy a [`Store`] enforces. See the module doc comment for
/// the full contrast; the short version is: `Local` migrates and saves,
/// silently. `Committed` migrates in memory and never saves on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// `.ivar/state.json`, lockfiles: local, gitignored, reproducible. A read
    /// that migrates persists the migrated form — nobody reviews this file, so
    /// there is nothing useful to say.
    Local,
    /// `ivar.json`: committed and team-shared. A read that migrates stays in
    /// memory only. Advancing the on-disk version is [`Store::migrate`]'s job,
    /// triggered by a human running an explicit command.
    Committed,
}

/// The function a [`Migration`] runs: transform the raw value forward one
/// step. Returns `Err` with a human-readable reason if the input does not have
/// the shape this step expects.
pub type MigrateFn = fn(serde_json::Value) -> Result<serde_json::Value, String>;

/// One step of a migration chain, from `from_version` to `to_version`.
///
/// Operates on [`serde_json::Value`] rather than any particular `T`, because a
/// step generally runs on data that does not parse as the *current* shape
/// yet — that mismatch is exactly why the step exists.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    from_version: u32,
    to_version: u32,
    migrate: MigrateFn,
}

impl Migration {
    /// A migration step. Chain-level invariants (starts at 0, contiguous, ends
    /// at the store's `current`) are checked by [`Store::new`], not here — a
    /// single `Migration` has nothing to check in isolation beyond what its
    /// type already guarantees.
    #[must_use]
    pub fn new(from_version: u32, to_version: u32, migrate: MigrateFn) -> Self {
        Self {
            from_version,
            to_version,
            migrate,
        }
    }
}

/// What migration would happen to a file, without touching it.
///
/// Returned by [`Store::inspect`], which is deliberately the one operation on
/// [`Store`] that never fails on a too-new file — that is what makes it safe
/// for `doctor` and the migrate command to run on data `read` would refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspection {
    /// The file is already at the store's current version.
    Current,
    /// The file is older than the store's current version; a migration would
    /// advance `detected` to `current`.
    NeedsMigration { detected: u32, current: u32 },
    /// The file is newer than the store's current version. [`Store::read`] and
    /// [`Store::write`] would refuse this outright; `inspect` only reports it.
    TooNew { detected: u32, current: u32 },
}

impl Inspection {
    /// Whether this inspection found a migration that would run.
    #[must_use]
    pub fn needs_migration(&self) -> bool {
        matches!(self, Self::NeedsMigration { .. })
    }
}

/// Read the schema version out of a raw JSON value. A missing `version` field,
/// or one that is not a JSON number, is version 0 — unversioned data predates
/// versioning, and that is not an error.
#[must_use]
pub fn detect_version(value: &serde_json::Value) -> u32 {
    value
        .as_object()
        .and_then(|object| object.get("version"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(0)
}

/// Set `version` on a JSON object value to `version`. A no-op on anything that
/// is not an object (there should be nothing else by the time this is called,
/// since every migration and every `T` this module handles is object-shaped).
fn stamp_version(value: &mut serde_json::Value, version: u32) {
    if let serde_json::Value::Object(map) = value {
        map.insert("version".to_owned(), serde_json::Value::from(version));
    }
}

/// Panics — this is a programmer error, not a runtime one — unless `migrations`
/// starts at version 0, is contiguous, and its last step lands exactly on
/// `current`. An empty chain never violates any of this; see the module doc
/// comment on why that is `ivar.json`'s own case, not an oversight.
fn assert_chain_valid(migrations: &[Migration], current: u32) {
    let Some(first) = migrations.first() else {
        return;
    };
    assert_eq!(
        first.from_version, 0,
        "migration chain must start at version 0, but the first migration starts at v{}",
        first.from_version
    );

    for (previous, next) in migrations.iter().zip(migrations.iter().skip(1)) {
        assert_eq!(
            next.from_version, previous.to_version,
            "migration chain has a gap or overlap: v{} does not connect to v{}",
            previous.to_version, next.from_version
        );
    }

    if let Some(last) = migrations.last() {
        assert_eq!(
            last.to_version, current,
            "migration chain ends at v{}, but the store's current version is v{current}",
            last.to_version
        );
    }
}

/// A versioned on-disk file of `T`, detected, migrated, and (depending on
/// [`Policy`]) persisted through one machine. See the module doc comment for
/// the contract and for why `Policy` is a field here rather than a type
/// parameter.
pub struct Store<T> {
    path: Utf8PathBuf,
    migrations: Vec<Migration>,
    current: u32,
    policy: Policy,
    // `fn() -> T` rather than `T` so `Store<T>` needs nothing from `T` itself
    // (no `Send`, `Sync`, or `Debug` bound leaks from a value this type never
    // actually holds) and manual `Debug` below never needs `T: Debug` either.
    _marker: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Store<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("migrations", &self.migrations)
            .field("current", &self.current)
            .field("policy", &self.policy)
            .finish()
    }
}

impl<T> Store<T> {
    /// Build a store over `path`, with an ordered `migrations` chain, at
    /// `current` version, under `policy`.
    ///
    /// # Panics
    ///
    /// If `migrations` is non-empty and does not start at version 0, is not
    /// contiguous, or does not end at `current`. See the module doc comment —
    /// these are programmer errors, caught here rather than on a user's disk.
    #[must_use]
    pub fn new(
        path: impl Into<Utf8PathBuf>,
        migrations: Vec<Migration>,
        current: u32,
        policy: Policy,
    ) -> Self {
        assert_chain_valid(&migrations, current);
        Self {
            path: path.into(),
            migrations,
            current,
            policy,
            _marker: PhantomData,
        }
    }

    /// The path this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    /// The migration policy this store enforces.
    #[must_use]
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// The current schema version — every migrated read and every write lands
    /// here.
    #[must_use]
    pub fn current(&self) -> u32 {
        self.current
    }
}

impl<T> Store<T>
where
    T: Serialize + DeserializeOwned,
{
    /// Read and, in memory, migrate the file to `current`. `Ok(None)` if it
    /// does not exist.
    ///
    /// Refuses (without modifying the file) if the on-disk version is newer
    /// than `current`. Under [`Policy::Local`], a migrated read persists the
    /// migrated form; under [`Policy::Committed`], the file is left exactly as
    /// it was.
    pub fn read(&self) -> Result<Option<T>, Error> {
        let Some((value, detected)) = self.read_migrated()? else {
            return Ok(None);
        };
        if detected < self.current && self.policy == Policy::Local {
            self.persist(&value)?;
        }
        Ok(Some(value))
    }

    /// Validate `value`, stamp it to `current`, and write it atomically.
    ///
    /// Refuses if the file already on disk is newer than `current` (for
    /// either policy — a version this binary does not understand must never
    /// be overwritten with one it does). Under [`Policy::Committed`],
    /// additionally refuses when the file on disk is *older* than `current`:
    /// that gap is only ever closed by an explicit [`Store::migrate`], never
    /// by a plain `write`.
    pub fn write(&self, value: &T) -> Result<(), Error> {
        if let Some(on_disk) = self.on_disk_version()? {
            self.guard_not_newer(on_disk)?;
            if self.policy == Policy::Committed && on_disk < self.current {
                return Err(Error::CommittedRefusesImplicitUpgrade {
                    path: self.path.clone(),
                    on_disk,
                    current: self.current,
                });
            }
        }
        self.persist(value)
    }

    /// What migration would happen, without mutating anything. `Ok(None)` if
    /// the file does not exist.
    ///
    /// Unlike [`read`](Self::read), this never refuses a too-new file — it
    /// reports [`Inspection::TooNew`] instead of erroring, because reporting
    /// safely on data this binary cannot open is the entire point of this
    /// method.
    pub fn inspect(&self) -> Result<Option<Inspection>, Error> {
        let Some(raw) = json::read::<serde_json::Value>(&self.path)? else {
            return Ok(None);
        };
        let detected = detect_version(&raw);
        Ok(Some(match detected.cmp(&self.current) {
            std::cmp::Ordering::Equal => Inspection::Current,
            std::cmp::Ordering::Less => Inspection::NeedsMigration {
                detected,
                current: self.current,
            },
            std::cmp::Ordering::Greater => Inspection::TooNew {
                detected,
                current: self.current,
            },
        }))
    }

    /// The explicit migrate path: read, migrate in memory, and persist the
    /// result — regardless of policy — if the file was not already at
    /// `current`. `Ok(None)` if the file does not exist.
    ///
    /// [`Policy::Local`] never needs this (every plain `read` already does
    /// it). [`Policy::Committed`] is the reason it exists: it is the one way
    /// a committed file's on-disk version ever advances.
    pub fn migrate(&self) -> Result<Option<T>, Error> {
        let Some((value, detected)) = self.read_migrated()? else {
            return Ok(None);
        };
        if detected < self.current {
            self.persist(&value)?;
        }
        Ok(Some(value))
    }

    /// Shared core of `read` and `migrate`: read the raw value, refuse if it
    /// is too new, run the migration chain, stamp the result to `current`, and
    /// deserialize into `T`. Returns the value and the version that was
    /// actually detected on disk, so callers can decide whether to persist.
    /// Never persists anything itself.
    fn read_migrated(&self) -> Result<Option<(T, u32)>, Error> {
        let Some(raw) = json::read::<serde_json::Value>(&self.path)? else {
            return Ok(None);
        };
        let detected = detect_version(&raw);
        self.guard_not_newer(detected)?;
        self.guard_has_migration_path(detected)?;

        let mut migrated = self.run_migrations(raw, detected)?;
        stamp_version(&mut migrated, self.current);
        let value = self.deserialize(migrated)?;

        Ok(Some((value, detected)))
    }

    /// The version currently on disk. `Ok(None)` if the file does not exist.
    fn on_disk_version(&self) -> Result<Option<u32>, Error> {
        let Some(raw) = json::read::<serde_json::Value>(&self.path)? else {
            return Ok(None);
        };
        Ok(Some(detect_version(&raw)))
    }

    /// Refuse if `detected` is newer than this store understands. This is the
    /// one refusal that applies identically on every path that touches the
    /// file — read or write, either policy.
    fn guard_not_newer(&self, detected: u32) -> Result<(), Error> {
        if detected > self.current {
            return Err(Error::TooNew {
                path: self.path.clone(),
                found: detected,
                highest: self.current,
            });
        }
        Ok(())
    }

    /// Whether a file detected at `detected` can actually reach `current`.
    ///
    /// Public because [`Inspection::NeedsMigration`] alone is not enough to
    /// promise a migration will run: with an empty chain there is nothing to
    /// migrate *with*, so a caller that previews "v0 → v1" from `inspect` and
    /// then calls [`migrate`](Self::migrate) would report a plan that refuses.
    /// A preview that can disagree with the act it previews is worse than no
    /// preview, so the reachability question is answerable without attempting
    /// the write.
    ///
    /// The chain invariants asserted in [`Store::new`] mean a non-empty chain
    /// always spans 0..=`current`, so the only way to have no path is an empty
    /// chain.
    #[must_use]
    pub fn has_migration_path(&self, detected: u32) -> bool {
        detected >= self.current || !self.migrations.is_empty()
    }

    /// Refuse data older than `current` that no migration can reach.
    ///
    /// See [`Error::NoMigrationPath`] for why that case is real and why the
    /// refusal lives here rather than in the caller.
    fn guard_has_migration_path(&self, detected: u32) -> Result<(), Error> {
        if !self.has_migration_path(detected) {
            return Err(Error::NoMigrationPath {
                path: self.path.clone(),
                found: detected,
                current: self.current,
            });
        }
        Ok(())
    }

    /// Run every migration step whose range covers `version`, in order,
    /// starting from `value`. A step outside `[from_version, to_version)` of
    /// `version` is skipped; the chain is walked once from front to back, not
    /// searched, because [`assert_chain_valid`] already guarantees it is
    /// contiguous. If any step fails, the accumulated value is dropped and the
    /// error names exactly which step — nothing produced so far is written
    /// anywhere, so the file on disk is never touched by a failed migration.
    fn run_migrations(
        &self,
        mut value: serde_json::Value,
        mut version: u32,
    ) -> Result<serde_json::Value, Error> {
        for migration in &self.migrations {
            if version < migration.from_version {
                break;
            }
            if version >= migration.to_version {
                continue;
            }
            value = (migration.migrate)(value).map_err(|reason| Error::MigrationFailed {
                path: self.path.clone(),
                from: migration.from_version,
                to: migration.to_version,
                reason,
            })?;
            version = migration.to_version;
        }
        Ok(value)
    }

    /// The last step before handing data to the caller: parse the fully
    /// migrated, version-stamped value as `T`. A mismatch here is not this
    /// module's business to explain beyond naming it — an unversioned or
    /// otherwise unmigratable file failing here is "rejected on schema
    /// grounds", exactly as the module doc comment promises.
    fn deserialize(&self, value: serde_json::Value) -> Result<T, Error> {
        serde_json::from_value(value).map_err(|source| Error::Deserialize {
            path: self.path.clone(),
            source,
        })
    }

    /// Serialize `value`, stamp it to `current`, and write it atomically
    /// through [`json::write_canonical`] — the only writer in the crate.
    fn persist(&self, value: &T) -> Result<(), Error> {
        let mut rendered = serde_json::to_value(value).map_err(Error::Serialize)?;
        stamp_version(&mut rendered, self.current);
        json::write_canonical(&self.path, &rendered)?;
        Ok(())
    }
}

/// Everything that can go wrong operating a [`Store`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading, parsing, or writing the underlying JSON failed. Delegates its
    /// `Failure` conversion entirely to the wrapped error — it already has its
    /// own code and fix action.
    #[error(transparent)]
    Json(#[from] json::Error),

    /// The file's schema version is newer than this binary understands. The
    /// one hard refusal: found on both `read` and `write`, and neither ever
    /// modifies the file when this fires.
    #[error(
        "{path} is at schema version {found}, but this build of ivar only understands up to version {highest}"
    )]
    TooNew {
        path: Utf8PathBuf,
        found: u32,
        highest: u32,
    },

    /// A [`Policy::Committed`] store was asked to `write` while the on-disk
    /// version is older than `current`. Advancing it is [`Store::migrate`]'s
    /// job, triggered explicitly, not a side effect of an ordinary write.
    #[error(
        "{path} is at schema version {on_disk}, older than the current version {current}; writing would silently advance a committed file"
    )]
    CommittedRefusesImplicitUpgrade {
        path: Utf8PathBuf,
        on_disk: u32,
        current: u32,
    },

    /// A migration step failed partway through the chain. Nothing was
    /// written — the failure happened entirely in memory, before any write
    /// was attempted.
    #[error("{path}: migrating from v{from} to v{to} failed: {reason}")]
    MigrationFailed {
        path: Utf8PathBuf,
        from: u32,
        to: u32,
        reason: String,
    },

    /// The fully migrated, version-stamped value does not match `T`'s shape.
    #[error("{path}: does not match the expected schema: {source}")]
    Deserialize {
        path: Utf8PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// The file is older than `current` and no migration covers it.
    ///
    /// With an empty chain and `current > 0` — a format whose first public
    /// version is not 0 — there is no v0 to migrate *from*, so a file detected
    /// at v0 is not this file at all. Without this guard `run_migrations` is a
    /// no-op and `stamp_version` would quietly relabel it as current, adopting
    /// a foreign file as one of ours. The published format contract says that
    /// must never happen, and the guard belongs here rather than in each
    /// caller that happens to have an empty chain.
    #[error(
        "{path} is at schema version {found}, and there is no migration to reach version {current}"
    )]
    NoMigrationPath {
        path: Utf8PathBuf,
        found: u32,
        current: u32,
    },

    /// `T` could not be converted to a JSON value at all (e.g. a map with
    /// non-string keys). Not the common case.
    #[error("could not serialize value to JSON: {0}")]
    Serialize(#[source] serde_json::Error),
}

impl Error {
    /// Stable, machine-matchable identifier for [`Failure::code`]. `Json`
    /// defers entirely to the wrapped error's own code, so it has none of its
    /// own here.
    const fn code(&self) -> &'static str {
        match self {
            Self::Json(_) => "store.json",
            Self::TooNew { .. } => "store.version_too_new",
            Self::CommittedRefusesImplicitUpgrade { .. } => {
                "store.committed_refuses_implicit_upgrade"
            }
            Self::MigrationFailed { .. } => "store.migration_failed",
            Self::NoMigrationPath { .. } => "store.no_migration_path",
            Self::Deserialize { .. } => "store.schema_mismatch",
            Self::Serialize(_) => "store.serialize_failed",
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // Both the code and the human sentence come off the error *before* it is
        // destructured. The `#[error(...)]` attribute is the single source of the
        // wording; re-typing it in each arm is how the two drift apart, and
        // rebuilding the variant just to ask for its code is worse still.
        let code = error.code();
        let what = error.to_string();

        match error {
            Error::Json(source) => source.into(),
            Error::TooNew { found, highest, .. } => Failure::blocked(code, what)
                .expected(format!("schema version {highest} or older"))
                .actual(format!("schema version {found}"))
                .fix(FixAction::unsafe_(
                    "store.upgrade_ivar",
                    format!("Upgrade ivar to a version that understands schema version {found}."),
                )),
            Error::CommittedRefusesImplicitUpgrade {
                on_disk, current, ..
            } => Failure::blocked(code, what)
                .expected(format!(
                    "schema version {current}, reached via an explicit migrate"
                ))
                .actual(format!("schema version {on_disk}"))
                .fix(
                    FixAction::unsafe_(
                        "store.run_migrate",
                        "Run the explicit migrate command — advancing a committed file's schema is a decision a human makes, not something ivar does on your behalf.",
                    )
                    .command("ivar migrate"),
                ),
            Error::MigrationFailed {
                from, to, reason, ..
            } => Failure::blocked(code, what)
                .expected(format!(
                    "v{from} data that the v{from}\u{2192}v{to} migration can transform"
                ))
                .actual(reason),
            Error::NoMigrationPath { found, current, .. } => Failure::blocked(code, what)
                .expected(format!("schema version {current}"))
                .actual(format!("schema version {found}, with no migration to reach it"))
                .fix(FixAction::unsafe_(
                    "store.not_our_file",
                    "Check this is the file you meant. A file at a version this format never had is not one ivar wrote.",
                )),
            Error::Deserialize { source, .. } => Failure::blocked(code, what)
                .expected("data matching the current schema")
                .actual(source.to_string()),
            Error::Serialize(_) => Failure::failed(code, what),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/versioned.rs"]
mod tests;
