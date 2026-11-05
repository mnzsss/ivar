//! `_consolidate.json`: the per-conversation markers and the last pass's outcome.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::infra::fs;
use crate::store::layout::Layout;

const STATE_FILE: &str = "_consolidate.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    pub last_id: i64,
    pub last_at: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsolidateState {
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    pub written_total: u64,
    pub markers: BTreeMap<String, Marker>,
}

#[must_use]
pub fn read_state(layout: &Layout) -> ConsolidateState {
    std::fs::read_to_string(layout.memory_spool_dir().join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn write_state(layout: &Layout, state: &ConsolidateState) -> Result<(), String> {
    let dir = layout.memory_spool_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write_atomic(&dir.join(STATE_FILE), &bytes).map_err(|error| error.to_string())
}
