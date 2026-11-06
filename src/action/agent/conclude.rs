//! Concluding an Agent Task: its file moves to `agents/done/` and an
//! `agent_task.concluded` observation goes to the delivery spool.

use std::io::Write;

use camino::Utf8PathBuf;
use serde_json::{Value, json};

use crate::action::memory::capture::sanitize_context;
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};
use crate::domain::agent::spec::AgentSpec;
use crate::domain::observation::{Importance, Observation, SourceEvent, agent_task_event_id};
use crate::domain::sanitize::Sanitized;
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::spool::{self, Appended, SpoolKey};

/// The observation body stays under the sanitiser's 4096-byte limit, so it is
/// never cut into invalid JSON.
const MAX_BODY_BYTES: usize = 4000;

/// Only a timer-fired run concludes a task, and a `skipped` one never ran: it
/// stays pending. A manual run is a rehearsal.
pub(crate) fn concludes(spec: &AgentSpec, receipt: &AgentRunReceipt) -> bool {
    spec.is_task()
        && receipt.trigger == Trigger::Schedule
        && receipt.outcome != AgentRunOutcome::Skipped
}

/// Moves the task's file to `agents/done/<name>.<at as YYYY-MM-DDTHHMM>.md`, or
/// `<name>.<at>.<run id>.md` when that name is taken. Never overwrites.
///
/// # Errors
///
/// `agent.task_not_concluded` when the file cannot be moved.
pub(crate) fn move_to_done(
    layout: &Layout,
    spec: &AgentSpec,
    run_id: &str,
) -> Result<Utf8PathBuf, Failure> {
    let Some(at) = spec.at else {
        return Err(not_concluded(spec, "it has no `at`"));
    };
    let done = layout.agents_done_dir();
    let stem = format!("{}.{}", spec.name, at.file_stamp());
    let mut target = done.join(format!("{stem}.md"));
    let moved = (|| -> Result<(), Failure> {
        fs::ensure_dir(&done)?;
        if fs::exists(&target)? {
            target = done.join(format!("{stem}.{run_id}.md"));
            if fs::exists(&target)? {
                return Err(Failure::failed(
                    "agent.task_not_concluded",
                    format!("{target} already exists and is never overwritten"),
                ));
            }
        }
        Ok(fs::rename(&layout.agent_file(&spec.name), &target)?)
    })();
    moved
        .map(|()| target)
        .map_err(|failure| not_concluded(spec, &failure.what))
}

fn not_concluded(spec: &AgentSpec, reason: &str) -> Failure {
    Failure::failed(
        "agent.task_not_concluded",
        format!(
            "Agent Task `{}` ran but its file was not moved to agents/done/: {reason}",
            spec.name
        ),
    )
}

fn tail_of(output: &str, chars: usize) -> &str {
    let Some(skip_before) = chars.checked_sub(1) else {
        return "";
    };
    match output.char_indices().rev().nth(skip_before) {
        Some((start, _)) => output.get(start..).unwrap_or(output),
        None => output,
    }
}

/// `{name, at, ran_at, outcome, run_id, output_tail}`, the tail cut from its front until it fits.
fn body(spec: &AgentSpec, receipt: &AgentRunReceipt) -> String {
    let at = spec.at.map(|at| at.to_string()).unwrap_or_default();
    let mut chars = receipt.output.chars().count().min(2000);
    loop {
        let body: Value = json!({
            "name": spec.name,
            "at": at,
            "ran_at": receipt.started_at,
            "outcome": receipt.outcome.as_str(),
            "run_id": receipt.run_id,
            "output_tail": tail_of(&receipt.output, chars),
        });
        let text = body.to_string();
        if text.len() <= MAX_BODY_BYTES || chars == 0 {
            return text;
        }
        chars /= 2;
    }
}

/// # Errors
///
/// When the body cannot be sanitised or the spool refuses the line.
pub(crate) fn record_concluded(
    layout: &Layout,
    spec: &AgentSpec,
    receipt: &AgentRunReceipt,
) -> Result<(), String> {
    let observation = Observation {
        event_id: agent_task_event_id(&spec.name, &receipt.run_id),
        source_event: SourceEvent::AgentTaskConcluded.as_str().to_owned(),
        occurred_at: rfc3339_now(),
        session_id: String::new(),
        native_session_id: String::new(),
        provider: String::new(),
        ivar_version: env!("CARGO_PKG_VERSION").to_owned(),
        feature: String::new(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::agent_task_concluded(receipt.outcome == AgentRunOutcome::Succeeded),
        attempt_id: receipt.run_id.clone(),
        body: body(spec, receipt),
    };
    let sanitized = Sanitized::sanitize(observation, &sanitize_context(layout))
        .map_err(|_| "the event body could not be sanitized".to_owned())?;
    match spool::append(
        &layout.memory_spool_dir(),
        &SpoolKey::delivery(),
        &sanitized,
    ) {
        Ok(Appended::Written) => Ok(()),
        Ok(Appended::Dropped) => Err("the delivery spool reached its size cap".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Records the event; a refusal prints one stderr line and changes nothing else (ADR-0009 D5).
pub(crate) fn report_concluded(layout: &Layout, spec: &AgentSpec, receipt: &AgentRunReceipt) {
    if let Err(reason) = record_concluded(layout, spec, receipt) {
        let _ = writeln!(
            std::io::stderr(),
            "ivar: memory.event_not_recorded: {reason}"
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/conclude.rs"]
mod tests;
