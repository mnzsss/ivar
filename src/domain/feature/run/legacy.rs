use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// One workstream copied out of an imported board, as evidence.
///
/// Deliberately flat strings: this is a *record of what the old file said*,
/// not a type the active domain reasons with. Nothing reads `status` to make
/// a decision — an imported receipt is already terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyWorkstream {
    /// The workstream's id in the old graph.
    pub id: String,
    /// Its human-readable title.
    pub title: String,
    /// Its last status on the board.
    pub status: String,
    /// The operations it was to run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<String>,
    /// The workstreams it waited on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// One journal entry copied out of an imported board, as evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyJournalEntry {
    /// Its total order on the board.
    pub seq: u64,
    /// When it was recorded, in the board's own format.
    pub timestamp: String,
    /// The workstream it was about; the board itself when empty.
    pub workstream: String,
    /// The kind of event.
    pub kind: String,
    /// The sentence a human reads.
    pub message: String,
}

/// Everything an imported board contributed to its receipt.
///
/// Immutable by convention and by use: nothing in the active lifecycle reads
/// or writes it after import. It exists so `status` can say *what was there*
/// without anyone having to open the archived board by hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyEvidence {
    /// SHA-256 of the normalized board this receipt was imported from. The
    /// import is idempotent because of this value: a crash between writing
    /// the receipt and archiving the board leaves both files on disk, and
    /// this is what says "the same import, continue" rather than "a different
    /// board, refuse".
    pub source_hash: String,
    /// The board's overall status at import.
    pub board_status: String,
    /// The plan fingerprint the board's graph was derived from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint: Option<String>,
    /// The board's workstreams, in graph order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workstreams: Vec<LegacyWorkstream>,
    /// The board's provider-session → workstream map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, String>,
    /// The board's journal, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub journal: Vec<LegacyJournalEntry>,
    /// Where the raw normalized board was archived.
    pub archived_board: Utf8PathBuf,
}
