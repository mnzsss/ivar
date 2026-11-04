//! The capture spool and the provider hooks that fill it. Read-only: nothing
//! is drained, renamed or written here.

use std::time::Duration;

use crate::action::memory::drain::read_state;
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::providers;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::spool;

use super::Diagnosis;

const ORPHAN_AFTER: Duration = Duration::from_secs(3600);

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
    if let Some(error) = read_state(layout).last_error {
        findings.push(Diagnosis {
            code: "memory.drain_failed",
            what: format!("the last drain of the capture spool failed: {error}"),
            fix: "Run `ivar memory status`; if the error repeats, read the database findings of `ivar doctor`.".to_owned(),
        });
    }
    findings
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
