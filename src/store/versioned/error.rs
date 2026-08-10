//! Everything that can go wrong operating a [`Store`](super::Store), and how
//! each becomes a user-facing [`Failure`].
//!
//! [`Error`] is the versioned store's own error type; its `Failure`
//! conversion is the single place a store problem acquires a code, an
//! expected/actual pair, and a fix action. The versioning machine itself —
//! [`Store`](super::Store), [`Migration`](super::Migration),
//! [`Policy`](super::Policy), [`Inspection`](super::Inspection) — stays in
//! `mod.rs`.

use camino::Utf8PathBuf;

use crate::error::{Failure, FixAction};
use crate::infra::json;

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
