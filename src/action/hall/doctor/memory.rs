//! The capture spool and the provider hooks that fill it. Read-only: nothing
//! is drained, renamed or written here.

use std::time::{Duration, SystemTime};

use crate::action::memory::drain::{DrainState, read_state};
use crate::action::memory::sweep::read_state as read_sweep_state;
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::providers;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::retention::{
    SESSION_RETENTION_DAYS, cutoff_before, oldest_session_last_observation,
};
use crate::store::memory::spool;

use super::Diagnosis;

const ORPHAN_AFTER: Duration = Duration::from_secs(3600);
const DROP_REPORTED_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
const RETENTION_GRACE_DAYS: u64 = 7;
const RETENTION_OVERDUE_AFTER_DAYS: u64 = SESSION_RETENTION_DAYS + RETENTION_GRACE_DAYS;

pub(super) fn memory_capture_diagnoses(layout: &Layout, manifest: &Manifest) -> Vec<Diagnosis> {
    let mut findings = spool_diagnoses(layout);
    if manifest.memory_capture() {
        findings.extend(missing_hook_diagnoses(layout, manifest));
    }
    findings
}

fn spool_diagnoses(layout: &Layout) -> Vec<Diagnosis> {
    let dir = layout.memory_spool_dir();
    let mut findings = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let waiting = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("jsonl" | "draining")
        );
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if waiting && age.is_some_and(|age| age > ORPHAN_AFTER) {
            findings.push(Diagnosis {
                code: "memory.spool_orphan",
                what: format!(
                    "`{}` has waited more than an hour to be drained",
                    path.display()
                ),
                fix: "Run `ivar memory status` to drain it.".to_owned(),
            });
        }
    }
    let dropped = spool::dropped_count(&dir);
    if dropped > 0 {
        findings.push(Diagnosis {
            code: "memory.spool_dropped",
            what: format!(
                "{dropped} events were dropped since the last drain because a session spool reached 8 MiB"
            ),
            fix: "Run `ivar memory status` (or keep `ivar graph mcp` running) so a drain acknowledges them; a session with no drain loses events above 8 MiB.".to_owned(),
        });
    }
    let state = read_state(layout);
    if dropped == 0 {
        findings.extend(acknowledged_drop_diagnosis(&state));
    }
    if let Some(error) = state.last_error {
        findings.push(Diagnosis {
            code: "memory.drain_failed",
            what: format!("the last drain of the capture spool failed: {error}"),
            fix: "Run `ivar memory status`; if the error repeats, read the database findings of `ivar doctor`.".to_owned(),
        });
    }
    if let Some(error) = read_sweep_state(layout).last_error {
        findings.push(Diagnosis {
            code: "memory.sweep_failed",
            what: format!("the last retention sweep failed: {error}"),
            fix: "Run `ivar memory status`; if the error repeats, read the database findings of `ivar doctor`.".to_owned(),
        });
    }
    if oldest_session_is_overdue(layout) {
        findings.push(Diagnosis {
            code: "memory.retention_overdue",
            what: format!("session observations older than {RETENTION_OVERDUE_AFTER_DAYS} days are still stored"),
            fix: "Keep `ivar graph mcp` running; its leader deletes expired session observations hourly.".to_owned(),
        });
    }
    findings
}

fn oldest_session_is_overdue(layout: &Layout) -> bool {
    let Some(oldest) = oldest_session_last_observation(layout.memory_db().as_std_path()) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    oldest < cutoff_before(now, RETENTION_OVERDUE_AFTER_DAYS)
}

fn acknowledged_drop_diagnosis(state: &DrainState) -> Option<Diagnosis> {
    let last_drop = state.dropped_last_epoch_secs?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    (now.saturating_sub(last_drop) <= DROP_REPORTED_FOR.as_secs()).then(|| Diagnosis {
        code: "memory.spool_dropped",
        what: format!(
            "{} events were dropped in total, most recently within 7 days, because a session spool reached 8 MiB",
            state.dropped_total
        ),
        fix: "Keep a drain running (`ivar graph mcp` or `ivar memory status`) and check how long sessions run; the finding clears after 7 days without new drops.".to_owned(),
    })
}

fn missing_hook_diagnoses(layout: &Layout, manifest: &Manifest) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    for provider in manifest.providers().available() {
        let present = match provider {
            Provider::ClaudeCode | Provider::T3 => {
                fs::read_text(&layout.root().join(Provider::CLAUDE_SETTINGS))
                    .ok()
                    .flatten()
                    .is_some_and(|text| text.contains("ivar memory capture"))
            }
            Provider::OpenCode | Provider::Omp => providers::capture_artifacts(*provider)
                .iter()
                .all(|artifact| layout.root().join(&artifact.relative_path).is_file()),
        };
        if !present {
            findings.push(Diagnosis {
                code: "provider.capture_hook_missing",
                what: format!(
                    "{provider} has no session capture hook although `memory.capture` is on"
                ),
                fix: "Run `ivar sync` to install it, or set `memory.capture` to false in `ivar.json`.".to_owned(),
            });
        }
    }
    findings
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/doctor/memory.rs"]
mod tests;
