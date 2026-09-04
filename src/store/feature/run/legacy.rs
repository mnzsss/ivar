//! Reading execution boards that were written before Run Receipts existed.
//!
//! This is the *whole* of what remains of the board format, and it is
//! permanent. N-COMPAT is a local-state contract, not a deprecation window:
//! `board.json` files exist on real machines, and a hall must open on every
//! future version of `ivar` regardless of how long ago the board actions were
//! deleted. So the v0 → v3 migration chain lives here, behind
//! [`normalize`], rather than beside a `Store<ExecutionBoard>` that is on its
//! way out.
//!
//! # Private DTOs, not the domain type
//!
//! Nothing here deserializes `ExecutionBoard`. The DTOs below are permissive by
//! construction — every field defaults, no field is denied, and `status` is a
//! plain `String` rather than an enum — because the job is to read whatever a
//! past `ivar` actually wrote, including a board whose status this binary has
//! no variant for. The active domain must not gain a reason to import board
//! types, so it does not see these.
//!
//! # What an import claims
//!
//! Nothing about continuity. A board that *completed* keeps its outcome; every
//! other board — running, blocked, paused, approved, never started — becomes
//! [`RunStatus::Interrupted`]. The old workstreams, their dependency waves,
//! their per-workstream sessions and write contracts have no faithful mapping
//! onto a provider-native coordinator, so the receipt records them as evidence
//! ([`LegacyEvidence`]) and claims only that they happened.
//!
//! # Crash safety
//!
//! Import is four steps and is restartable at every boundary between them:
//!
//! ```text
//! 1. board only              archive/boards/<hash>.json written
//! 2. receipt + board         run.json written
//! 3. receipt + archive + board   archive/runs/<id>.json written, run.json removed
//! 4. completed               board.json removed
//! ```
//!
//! Nothing is destroyed before its replacement is durable: the board archive is
//! written first, `board.json` is removed last, and each step is a no-op when
//! its output already exists. What links a resumed import to the one it is
//! resuming is [`LegacyEvidence::source_hash`] — the SHA-256 of the *normalized*
//! board — which is also what tells a continuation apart from a second, different
//! board arriving under a half-finished import.

use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::feature::{
    LegacyEvidence, LegacyJournalEntry, LegacyWorkstream, RunId, RunOutcome, RunProvenance,
    RunReceipt, RunStatus,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::infra::{fs, hash, json};
use crate::store::layout::Layout;
use crate::store::versioned::{MigrateFn, detect_version};

/// The filename a feature's execution board lives in, under its execution
/// directory. Owned here rather than by `store::feature` so it outlives the
/// board store it used to belong to.
const BOARD_FILE: &str = "board.json";

/// The last schema version `board.json` was ever written at. The chain below
/// ends here and never moves again — no board will be written in this format
/// after this feature lands.
const BOARD_VERSION: u32 = 3;

/// The path of a feature's execution board file.
#[must_use]
pub(in crate::store::feature) fn board_path(layout: &Layout, feature: &FeatureName) -> Utf8PathBuf {
    layout.execution_dir(feature).join(BOARD_FILE)
}

/// What one import did.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The receipt the board became. Terminal, always.
    pub receipt: RunReceipt,
    /// Where the raw normalized board was archived.
    pub archived_board: Utf8PathBuf,
    /// Whether this call finished an import an earlier one had started, rather
    /// than performing one from scratch. Reported so `status` and `start` can
    /// say "resumed" instead of silently inventing a second import.
    pub resumed: bool,
}

/// Turn a feature's legacy `board.json` into an archived, terminal Run Receipt.
///
/// `Ok(None)` when there is no board — which is both "this feature never had
/// one" and "a previous import already completed", and neither needs
/// distinguishing: the receipt is already in the archive either way.
///
/// `id` is only used when a fresh receipt has to be minted. A resumed import
/// reuses the id the first attempt recorded, so restarting never doubles a run
/// in the history.
pub fn import(
    layout: &Layout,
    feature: &FeatureName,
    plan_path: impl Into<Utf8PathBuf>,
    id: RunId,
    at: &str,
) -> Result<Option<Import>, Failure> {
    let board = board_path(layout, feature);
    let Some(raw) = json::read::<serde_json::Value>(&board)? else {
        return Ok(None);
    };

    let normalized = normalize(&board, raw)?;
    let canonical = json::to_canonical_string(&normalized)?;
    let source_hash = hash::text(&canonical);

    let existing = find_import(layout, feature, &source_hash)?;
    guard_conflict(layout, feature, &source_hash)?;

    // Step 1 — the raw board, content-addressed. Written before anything reads
    // as imported, so the evidence a receipt points at is on disk before the
    // receipt claiming it is.
    let archived_board = archive_board(layout, feature, &source_hash, &canonical)?;

    let resumed = existing.is_some();
    let receipt = match existing {
        Some(receipt) => receipt,
        None => {
            let evidence = evidence(&normalized, source_hash, archived_board.clone())?;
            let (status, outcome) = outcome_of(&evidence.board_status);
            RunReceipt::from_legacy(
                id,
                feature.clone(),
                plan_path,
                status,
                outcome,
                evidence,
                at,
            )
        }
    };

    // Step 2 — the receipt as the current run, then step 3, which archives it
    // and clears `run.json`. Two steps rather than one write straight into the
    // archive, so a crash between them leaves a state the next call recognises.
    if super::RunReceipt::read_archived(layout, feature, &receipt.id)?.is_none() {
        receipt.write(layout)?;
    }
    super::archive(layout, &receipt)?;
    let current = super::current_path(layout, feature);
    if fs::exists(&current)? {
        fs::remove_file(&current)?;
    }

    // Step 4 — the board is now fully represented by an archive and a receipt,
    // and only now may it go.
    fs::remove_file(&board)?;

    Ok(Some(Import {
        receipt,
        archived_board,
        resumed,
    }))
}

/// The already-imported receipt for `source_hash`, wherever a half-finished
/// import left it: the current `run.json`, or the archive.
fn find_import(
    layout: &Layout,
    feature: &FeatureName,
    source_hash: &str,
) -> Result<Option<RunReceipt>, Failure> {
    if let Some(current) = RunReceipt::read(layout, feature)?
        && imported_from(&current, source_hash)
    {
        return Ok(Some(current));
    }
    Ok(super::history(layout, feature)?
        .into_iter()
        .find(|receipt| imported_from(receipt, source_hash)))
}

/// Whether `receipt` is the legacy import of the board hashing to
/// `source_hash`.
fn imported_from(receipt: &RunReceipt, source_hash: &str) -> bool {
    receipt.provenance == RunProvenance::LegacyImport
        && receipt
            .legacy
            .as_ref()
            .is_some_and(|legacy| legacy.source_hash == source_hash)
}

/// Refuse to import over a run that is not this import's own.
///
/// Two shapes are refused. A **live native run** means a coordinator is holding
/// this feature right now, and writing an imported receipt into `run.json`
/// would take the lock out from under it. A **different legacy import** means
/// `board.json` changed after an import began — the source hash no longer
/// matches the receipt sitting in `run.json` — and continuing would silently
/// merge two boards into one history.
fn guard_conflict(
    layout: &Layout,
    feature: &FeatureName,
    source_hash: &str,
) -> Result<(), Failure> {
    let Some(current) = RunReceipt::read(layout, feature)? else {
        return Ok(());
    };
    if imported_from(&current, source_hash) {
        return Ok(());
    }

    if current.provenance == RunProvenance::LegacyImport {
        return Err(Failure::blocked(
            "execute.legacy_source_conflict",
            format!(
                "{} was imported from a different board than the one on disk now",
                super::current_path(layout, feature)
            ),
        )
        .expected(format!("a board hashing to {source_hash}"))
        .actual(format!(
            "a receipt imported from {}",
            current
                .legacy
                .as_ref()
                .map_or("an unrecorded board", |legacy| legacy.source_hash.as_str())
        ))
        .fix(FixAction::unsafe_(
            "execute.remove_stale_board",
            format!(
                "The import that produced run {} is complete. Move {} aside by hand to \
                 confirm the newer board should be discarded.",
                current.id,
                board_path(layout, feature)
            ),
        )));
    }

    if current.holds_lock() {
        return Err(Failure::blocked(
            "execute.legacy_import_blocked",
            format!(
                "run {} is {} — a legacy board cannot be imported while a run is in flight",
                current.id, current.status
            ),
        )
        .expected("no run in flight")
        .actual(current.status.to_string())
        .fix(FixAction::safe(
            "execute.finish_run",
            format!(
                "Finish or abandon run {} first: `ivar feature execute finish {feature}` or \
                 `ivar feature execute start {feature} --plan <path> --restart`.",
                current.id
            ),
        )));
    }
    Ok(())
}

/// Write the raw normalized board to its content-addressed path.
///
/// Content-addressed means identical content lands on the same path, so a
/// re-run writes the same bytes to the same place. Different content computes a
/// different name and therefore cannot collide — the equality check below is
/// the assertion that this still holds, not a merge strategy.
fn archive_board(
    layout: &Layout,
    feature: &FeatureName,
    source_hash: &str,
    canonical: &str,
) -> Result<Utf8PathBuf, Failure> {
    let path = layout.archived_board(feature, source_hash);
    if let Some(existing) = fs::read_text(&path)? {
        if existing.trim_end() == canonical.trim_end() {
            return Ok(path);
        }
        return Err(Failure::blocked(
            "execute.board_archive_conflict",
            format!("{path} already holds different content"),
        )
        .expected("a content-addressed archive to match its own hash")
        .actual("different bytes under the same hash".to_owned())
        .fix(FixAction::unsafe_(
            "execute.inspect_board_archive",
            format!("Move {path} aside by hand after checking what it holds."),
        )));
    }
    fs::ensure_dir(&layout.board_archive_dir(feature))?;
    json::write_canonical(&path, &canonical_value(canonical)?)?;
    Ok(path)
}

/// Re-parse the canonical string so `write_canonical` writes exactly the bytes
/// the source hash was taken over, rather than a re-serialization of a
/// differently-ordered value.
fn canonical_value(canonical: &str) -> Result<serde_json::Value, Failure> {
    serde_json::from_str(canonical).map_err(|source| {
        Failure::failed(
            "execute.board_archive_unreadable",
            format!("the normalized board did not round-trip through JSON: {source}"),
        )
    })
}

/// The status and outcome a board's own status becomes.
///
/// Only two boards are terminal in the honest sense: one that completed and one
/// that failed. Everything else stopped without reporting, and
/// [`RunStatus::Interrupted`] is what "stopped without a reported outcome"
/// means — an unknown status from a future-shaped board lands there too, which
/// is the safe default rather than a parse failure.
fn outcome_of(board_status: &str) -> (RunStatus, Option<RunOutcome>) {
    match board_status {
        "completed" => (RunStatus::Succeeded, Some(RunOutcome::Succeeded)),
        "failed" => (RunStatus::Failed, Some(RunOutcome::Failed)),
        _ => (RunStatus::Interrupted, None),
    }
}

/// Everything the board contributes to its receipt.
fn evidence(
    normalized: &serde_json::Value,
    source_hash: String,
    archived_board: Utf8PathBuf,
) -> Result<LegacyEvidence, Failure> {
    let board: BoardDto = serde_json::from_value(normalized.clone()).map_err(|source| {
        Failure::blocked(
            "execute.board_unreadable",
            format!("the execution board does not match any shape ivar has written: {source}"),
        )
        .fix(FixAction::unsafe_(
            "execute.inspect_board",
            "Open the board and check it against the format ivar last wrote.",
        ))
    })?;

    Ok(LegacyEvidence {
        source_hash,
        board_status: board.status,
        plan_fingerprint: board
            .graph
            .plan_fingerprint
            .filter(|fingerprint| !fingerprint.is_empty()),
        workstreams: board
            .graph
            .workstreams
            .into_iter()
            .map(|workstream| LegacyWorkstream {
                id: workstream.id,
                title: workstream.title,
                status: workstream.status,
                operations: workstream.operations,
                depends_on: workstream.depends_on,
            })
            .collect(),
        sessions: board.sessions,
        journal: board
            .journal
            .into_iter()
            .map(|entry| LegacyJournalEntry {
                seq: entry.seq,
                timestamp: entry.timestamp,
                workstream: entry.workstream,
                kind: entry.kind,
                message: entry.message,
            })
            .collect(),
        archived_board,
    })
}

/// Bring a raw board value up to [`BOARD_VERSION`], whatever version it was
/// written at.
///
/// A board newer than this binary is refused rather than guessed at — the same
/// rule the versioned store applies, stated here because this path deliberately
/// does not go through a `Store<T>`: there is no `T` left to deserialize into.
fn normalize(
    path: &camino::Utf8Path,
    mut value: serde_json::Value,
) -> Result<serde_json::Value, Failure> {
    let detected = detect_version(&value);
    if detected > BOARD_VERSION {
        return Err(Failure::blocked(
            "store.version_too_new",
            format!(
                "{path} is schema v{detected}; this ivar understands execution boards up to v{BOARD_VERSION}"
            ),
        )
        .expected(format!("v{BOARD_VERSION} or older"))
        .actual(format!("v{detected}"))
        .fix(FixAction::safe(
            "store.upgrade_ivar",
            "Upgrade ivar to a version that understands this hall.",
        )));
    }

    for (from, to, step) in STEPS {
        if detected <= from {
            value = step(value).map_err(|reason| {
                Failure::blocked(
                    "store.migration_failed",
                    format!("{path}: migrating from v{from} to v{to} failed: {reason}"),
                )
            })?;
        }
    }
    if let serde_json::Value::Object(map) = &mut value {
        map.insert("version".to_owned(), serde_json::Value::from(BOARD_VERSION));
    }
    Ok(value)
}

/// The board's migration chain, in order. Contiguous from v0 to
/// [`BOARD_VERSION`] and never pruned.
const STEPS: [(u32, u32, MigrateFn); 3] = [(0, 1, v0_to_v1), (1, 2, v1_to_v2), (2, 3, v2_to_v3)];

/// Migrate a board.json from v0 → v1. The board has never had a v0 shape: it
/// has been written with `version: 1` since the day it shipped, like
/// `ivar.json` itself. The step exists to keep the chain contiguous — a file
/// with no `version` field at all is treated as v1 and passed through.
pub(in crate::store::feature) fn v0_to_v1(
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(value)
}

/// Migrate a board.json from v1 → v2.
///
/// v2 keeps v1's shape — `status`, `graph {workstreams, plan_fingerprint}`,
/// `journal` — and adds fields with sensible defaults: `next_event_seq` and
/// `seq`/`event_id` on the journal (the monotonic order and identity that made
/// tick/reply idempotent), `blocked_by` and `sessions` on the board, and
/// `provider`/`agent` on each workstream.
pub(in crate::store::feature) fn v1_to_v2(
    mut value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("board must be an object")?;

    // --- workstreams: add provider/agent where missing ----------------------
    let graph = root
        .get_mut("graph")
        .and_then(|g| g.as_object_mut())
        .ok_or("board is missing graph")?;
    if let Some(streams) = graph.get_mut("workstreams").and_then(|w| w.as_array_mut()) {
        for ws in streams {
            let obj = ws.as_object_mut().ok_or("workstream not an object")?;
            obj.entry("provider").or_insert(serde_json::Value::Null);
            obj.entry("agent").or_insert(serde_json::Value::Null);
        }
    }

    // --- journal: number the entries with seq/event_id ----------------------
    let mut fallback = Vec::new();
    let journal = root
        .get_mut("journal")
        .and_then(|j| j.as_array_mut())
        .unwrap_or(&mut fallback);
    for (index, entry) in journal.iter_mut().enumerate() {
        let obj = entry.as_object_mut().ok_or("journal entry not an object")?;
        let seq = (index + 1) as u64;
        let kind = obj
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| "unknown".to_owned());
        obj.entry("seq").or_insert(serde_json::Value::from(seq));
        obj.entry("event_id")
            .or_insert_with(|| serde_json::Value::String(format!("migrated.v1.{kind}.{seq}")));
    }
    let next_seq = (journal.len() + 1) as u64;

    // --- board: new fields with sensible defaults ---------------------------
    root.entry("next_event_seq")
        .or_insert(serde_json::Value::from(next_seq));
    root.entry("blocked_by").or_insert(serde_json::Value::Null);
    root.entry("sessions")
        .or_insert(serde_json::Value::Object(Default::default()));

    Ok(value)
}

/// Migrate a board.json from v2 → v3.
///
/// v3 added `revision: Option<String>` to every journal entry — the plan
/// fingerprint an entry satisfied. The step fills the explicit `null` so the
/// normalized v3 is the canonical shape, while rewriting nothing else: every
/// existing field and the entry order are left byte-for-byte untouched.
pub(in crate::store::feature) fn v2_to_v3(
    mut value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("board must be an object")?;
    let mut fallback = Vec::new();
    let journal = root
        .get_mut("journal")
        .and_then(|j| j.as_array_mut())
        .unwrap_or(&mut fallback);
    for entry in journal.iter_mut() {
        let obj = entry.as_object_mut().ok_or("journal entry not an object")?;
        obj.entry("revision").or_insert(serde_json::Value::Null);
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// permissive DTOs
// ---------------------------------------------------------------------------

/// A board, read as loosely as it can be read.
///
/// No `deny_unknown_fields` and no enums: a board written by a past `ivar` is
/// data to be preserved, not a shape to be validated. Every field defaults, so
/// a board missing half of them still imports the half it has.
#[derive(Debug, Default, Deserialize)]
struct BoardDto {
    #[serde(default)]
    status: String,
    #[serde(default)]
    graph: GraphDto,
    #[serde(default)]
    journal: Vec<JournalDto>,
    #[serde(default)]
    sessions: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct GraphDto {
    #[serde(default)]
    workstreams: Vec<WorkstreamDto>,
    #[serde(default)]
    plan_fingerprint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct WorkstreamDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    operations: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct JournalDto {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    workstream: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    message: String,
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/feature/run/legacy.rs"]
mod tests;
