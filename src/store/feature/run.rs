//! Run Receipt persistence: one current receipt, an immutable archive, and the
//! history the two make together.
//!
//! ```text
//! features/<feature>/execution/run.json                    the current run
//! features/<feature>/execution/archive/runs/<run-id>.json   every finished run
//! features/<feature>/execution/archive/boards/<hash>.json   imported legacy boards
//! ```
//!
//! # One current, many archived
//!
//! At most one `run.json` exists, and while it is non-terminal it *is* the
//! feature's single-run lock — [`RunStatus::holds_lock`](crate::domain::feature::RunStatus::holds_lock)
//! is the whole of that check, and it is why a second coordinator is refused
//! rather than racing the first. A run that reaches a terminal state is moved
//! whole into `archive/runs/<run-id>.json` and `run.json` is removed, which is
//! what makes room for the next one. Nothing is ever deleted: `feature close`
//! keeps execution history, so the archive only grows.
//!
//! # Why archiving refuses rather than overwrites
//!
//! An archived receipt is evidence. [`archive`] writes a run id that already
//! exists only when the content is byte-identical — a re-run of an interrupted
//! step continues, and anything else is refused. That is not defensiveness for
//! its own sake: legacy import (in [`legacy`]) is deliberately restartable at
//! every crash point, and "the same archive, again" has to be distinguishable
//! from "a different run under an id already spoken for".
//!
//! # Paths
//!
//! Every path here comes from [`Layout`]. Actions receive computed paths and
//! never join a filename, so the on-disk shape is this module's to change.

pub(super) mod legacy;

use camino::Utf8PathBuf;

use crate::domain::feature::{RUN_CURRENT_VERSION, RunId, RunReceipt};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::versioned::{Policy, Store};

pub use legacy::{Import, import};

/// The suffix every receipt and board archive file carries. Anything else in
/// the archive directory is not ours — an editor swap file, a stray copy — and
/// [`history`] steps over it rather than failing the whole listing.
const JSON_SUFFIX: &str = ".json";

impl RunReceipt {
    /// Read `features/<feature>/execution/run.json` — the current run.
    /// `Ok(None)` when the feature has no run in flight and none waiting to be
    /// archived.
    ///
    /// A file newer than this binary understands is a hard error; see
    /// [`Store::read`].
    pub fn read(layout: &Layout, feature: &FeatureName) -> Result<Option<Self>, Failure> {
        current_store(layout, feature).read().map_err(Failure::from)
    }

    /// Write this receipt to `features/<feature>/execution/run.json`,
    /// atomically, in canonical form. Creates the execution directory if it
    /// does not exist.
    ///
    /// Takes the feature from the receipt rather than as an argument: a
    /// receipt names the feature it executes, and letting a caller pass a
    /// different one is how a run lands under the wrong feature.
    pub fn write(&self, layout: &Layout) -> Result<(), Failure> {
        fs::ensure_dir(&layout.execution_dir(&self.feature))?;
        current_store(layout, &self.feature)
            .write(self)
            .map_err(Failure::from)
    }

    /// Read one archived receipt by id. `Ok(None)` when no run with that id
    /// has been archived — which includes the id of the *current* run.
    pub fn read_archived(
        layout: &Layout,
        feature: &FeatureName,
        id: &RunId,
    ) -> Result<Option<Self>, Failure> {
        archive_store(layout, feature, id)
            .read()
            .map_err(Failure::from)
    }

    /// Find one receipt by id, wherever it lives: the current run first, then
    /// the archive.
    ///
    /// Current-first because the current receipt is the one that changes, and
    /// an id that is both current and archived would mean an archive was
    /// written before the run ended — a bug this ordering reports as the live
    /// value rather than a stale copy.
    pub fn find(
        layout: &Layout,
        feature: &FeatureName,
        id: &RunId,
    ) -> Result<Option<Self>, Failure> {
        if let Some(current) = Self::read(layout, feature)?
            && &current.id == id
        {
            return Ok(Some(current));
        }
        Self::read_archived(layout, feature, id)
    }
}

/// The path of a feature's current Run Receipt. Public because the action
/// layer names it in its outcomes and its failure messages; the filename stays
/// this module's to own, so no path arithmetic leaks into `action`.
#[must_use]
pub fn current_path(layout: &Layout, feature: &FeatureName) -> Utf8PathBuf {
    layout.run_receipt(feature)
}

/// The path one archived receipt lives at.
#[must_use]
pub fn archive_path(layout: &Layout, feature: &FeatureName, id: &RunId) -> Utf8PathBuf {
    layout.archived_run(feature, id)
}

/// Copy a terminal receipt into the archive, and answer where it landed.
///
/// Refuses a non-terminal receipt: the archive is for runs that are over, and
/// a live run copied into it would be a second, frozen truth about a run still
/// changing.
///
/// Writing an id that already holds *identical* content is a no-op rather than
/// an error, which is what makes the callers that archive-then-remove
/// restartable after a crash between the two steps. Different content under an
/// existing id is refused outright — never overwritten.
pub fn archive(layout: &Layout, receipt: &RunReceipt) -> Result<Utf8PathBuf, Failure> {
    if !receipt.status.is_terminal() {
        return Err(Failure::blocked(
            "execute.archive_live_run",
            format!(
                "run {} is {} and cannot be archived",
                receipt.id, receipt.status
            ),
        )
        .expected("a run that has succeeded, failed, or been interrupted")
        .actual(receipt.status.to_string())
        .fix(FixAction::safe(
            "execute.finish_run",
            "Finish the run with `ivar feature execute finish <feature>`, or abandon it with \
             `ivar feature execute start <feature> --plan <path> --restart`.",
        )));
    }

    let path = archive_path(layout, &receipt.feature, &receipt.id);
    if let Some(existing) = RunReceipt::read_archived(layout, &receipt.feature, &receipt.id)? {
        if &existing == receipt {
            return Ok(path);
        }
        return Err(Failure::blocked(
            "execute.archive_conflict",
            format!(
                "run {} is already archived with different content",
                receipt.id
            ),
        )
        .expected("an archive entry identical to the receipt being archived")
        .actual(format!("a different receipt already at {path}"))
        .fix(FixAction::unsafe_(
            "execute.inspect_archived_run",
            format!(
                "Inspect both with `ivar feature execute status {} --run {}` and move {path} \
                 aside by hand if the archived copy is wrong.",
                receipt.feature, receipt.id
            ),
        )));
    }

    fs::ensure_dir(&layout.run_archive_runs_dir(&receipt.feature))?;
    archive_store(layout, &receipt.feature, &receipt.id)
        .write(receipt)
        .map_err(Failure::from)?;
    Ok(path)
}

/// Archive the current receipt and clear `run.json`, making room for the next
/// run. `Ok(None)` when there is no current receipt at all.
///
/// The order is archive-then-remove, never the reverse: a crash between the
/// two leaves the receipt in both places, and the next call finds an identical
/// archive entry, no-ops on it, and removes `run.json` again. A crash the
/// other way round would lose the run.
pub fn archive_current(layout: &Layout, feature: &FeatureName) -> Result<Option<RunId>, Failure> {
    let Some(receipt) = RunReceipt::read(layout, feature)? else {
        return Ok(None);
    };
    archive(layout, &receipt)?;
    fs::remove_file(&current_path(layout, feature))?;
    Ok(Some(receipt.id))
}

/// Every receipt this feature has, newest first.
///
/// The current run is included — a user asking for history wants the whole
/// picture, not the archive minus whatever is in flight. Ordered by start time
/// with the run id breaking ties, so two listings of the same directory read
/// identically no matter what order the filesystem hands the entries back.
///
/// Files in the archive directory that are not `.json` are stepped over. A
/// `.json` file that does not read as a receipt is *not* stepped over — a
/// corrupt archive entry is a run silently vanishing from an audit trail, and
/// that is worth failing loudly for.
pub fn history(layout: &Layout, feature: &FeatureName) -> Result<Vec<RunReceipt>, Failure> {
    let mut receipts = Vec::new();
    if let Some(current) = RunReceipt::read(layout, feature)? {
        receipts.push(current);
    }

    let dir = layout.run_archive_runs_dir(feature);
    if fs::is_dir(&dir)? {
        for path in fs::read_dir(&dir)? {
            let Some(name) = path.file_name() else {
                continue;
            };
            let Some(raw) = name.strip_suffix(JSON_SUFFIX) else {
                continue;
            };
            let Ok(id) = RunId::new(raw) else {
                continue;
            };
            if let Some(receipt) = RunReceipt::read_archived(layout, feature, &id)? {
                receipts.push(receipt);
            }
        }
    }

    receipts.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(receipts)
}

/// The versioned store over a feature's current receipt.
///
/// The chain is empty and `current` is 1 on purpose: `run.json` has never had
/// an unversioned predecessor, so — exactly as with `ivar.json` — there is no
/// v0 → v1 step to write, and a file with no `version` field is refused on
/// schema grounds rather than adopted. Every future schema change adds its step
/// here and none is ever pruned.
fn current_store(layout: &Layout, feature: &FeatureName) -> Store<RunReceipt> {
    Store::new(
        layout.run_receipt(feature),
        Vec::new(),
        RUN_CURRENT_VERSION,
        Policy::Local,
    )
}

/// The versioned store over one archived receipt. Same schema and same chain as
/// the current one — an archived receipt is the same value, moved.
fn archive_store(layout: &Layout, feature: &FeatureName, id: &RunId) -> Store<RunReceipt> {
    Store::new(
        layout.archived_run(feature, id),
        Vec::new(),
        RUN_CURRENT_VERSION,
        Policy::Local,
    )
}
