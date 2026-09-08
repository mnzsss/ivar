//! Store operations for feature memory handoffs.

use crate::domain::memory::handoff::HandoffPayload;
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;

/// Persist a handoff payload to the feature inbox.
///
/// Writes JSON to `<hall>/.ivar/features/<feature>/memory/inbox/<handoff.id>.json`.
pub fn persist_handoff(
    layout: &Layout,
    feature: &FeatureName,
    handoff: &HandoffPayload,
) -> Result<(), Failure> {
    let inbox_dir = layout.feature_memory_inbox(feature);
    fs::ensure_dir(&inbox_dir)?;
    let target_file = inbox_dir.join(format!("{}.json", handoff.id));

    let json_content = serde_json::to_string_pretty(handoff).map_err(|e| {
        Failure::failed(
            "memory.handoff_serialize",
            format!("failed to serialize handoff {}: {e}", handoff.id),
        )
    })?;

    fs::write_text(&target_file, &json_content)?;
    Ok(())
}

/// Atomically claim all pending handoffs from the feature memory inbox,
/// moving them into the feature memory archive.
///
/// Returns all claimed handoffs sorted deterministically by ID.
pub fn claim_pending_handoffs(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<HandoffPayload>, Failure> {
    let inbox_dir = layout.feature_memory_inbox(feature);
    if !fs::is_dir(&inbox_dir)? {
        return Ok(Vec::new());
    }

    let archive_dir = layout.feature_memory_archive(feature);
    fs::ensure_dir(&archive_dir)?;

    let mut claimed = Vec::new();
    let entries = fs::read_dir(&inbox_dir)?;

    for entry in entries {
        let Some(file_name) = entry.file_name().map(|s| s.to_string()) else {
            continue;
        };
        if !file_name.ends_with(".json") {
            continue;
        }

        let source_path = entry;
        let target_path = archive_dir.join(&file_name);
        // Try moving. Atomic rename:
        // If rename succeeds, read from target_path. If rename fails because already moved (race), skip.
        if let Err(err) = fs::rename(&source_path, &target_path) {
            // Check if source still exists
            if !fs::exists(&source_path)? {
                continue;
            }
            return Err(Failure::from(err));
        }

        if let Some(content) = fs::read_text(&target_path)? {
            match serde_json::from_str::<HandoffPayload>(&content) {
                Ok(payload) => claimed.push(payload),
                Err(_) => {}
            }
        }
    }

    claimed.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(claimed)
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/handoff.rs"]
mod tests;
