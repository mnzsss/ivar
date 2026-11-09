//! The hall scripts an agent lists in `scripts:`: the pre-launch approval
//! check and the `# env:` check `ivar agent sync` warns with.
//!
//! A script is approved while its current SHA-256 equals the one `ivar script
//! approve` recorded for its hall-relative path. The agent file only names the
//! scripts; approving one is always a human act in an interactive session.

use crate::domain::agent::spec::AgentSpec;
use crate::domain::script;
use crate::domain::script_approval::state;
use crate::error::{Failure, Warning};
use crate::infra::{fs, hash};
use crate::store::layout::Layout;
use crate::store::memory::approvals;

/// The reason an unreviewed script blocks a run: the code a receipt's output
/// and the notification open with.
pub const UNREVIEWED: &str = "agent.script_unreviewed";

/// Refuses the run when any script in `spec.scripts` is missing, never
/// approved, or changed since its approval. Reads `memory.db` without creating
/// or migrating it.
///
/// # Errors
///
/// `agent.script_unreviewed` naming every such path and how to approve it, or
/// `script.approvals_unavailable` when `memory.db` cannot be read.
pub fn ensure_approved(layout: &Layout, spec: &AgentSpec) -> Result<(), Failure> {
    if spec.scripts.is_empty() {
        return Ok(());
    }
    let recorded = approvals::read_all(layout.memory_db().as_std_path()).map_err(|error| {
        Failure::failed(
            "script.approvals_unavailable",
            format!("could not use the script approvals in memory.db: {error}"),
        )
    })?;
    let mut unreviewed = Vec::new();
    for path in &spec.scripts {
        let reason = match fs::read_bytes(&layout.root().join(path))? {
            None => "missing",
            Some(bytes) => {
                match state(recorded.get(path).map(String::as_str), &hash::bytes(&bytes)).reason() {
                    None => continue,
                    Some(reason) => reason.as_str(),
                }
            }
        };
        unreviewed.push(format!("`{path}` ({reason})"));
    }
    if unreviewed.is_empty() {
        return Ok(());
    }
    Err(Failure::failed(
        UNREVIEWED,
        format!(
            "{UNREVIEWED}: not launched, unreviewed script: {}. Approve it with `ivar script approve <path>` in an interactive session; the next run proceeds.",
            unreviewed.join(", ")
        ),
    ))
}

/// A warning for each listed script whose `# env:` header names a variable the
/// agent passes neither in `secrets` nor in `env`: the script would run
/// without it.
#[must_use]
pub fn env_warnings(layout: &Layout, specs: &[AgentSpec]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for spec in specs {
        for path in &spec.scripts {
            let Some(name) = path.rsplit('/').next() else {
                continue;
            };
            let Ok(Some(source)) = fs::read_text(&layout.root().join(path)) else {
                continue;
            };
            let Ok(header) = script::parse(name, &source) else {
                continue;
            };
            let missing: Vec<String> = header
                .env
                .into_iter()
                .filter(|var| !spec.secrets.contains(var) && !spec.env.contains_key(var))
                .map(|var| format!("`{var}`"))
                .collect();
            if !missing.is_empty() {
                warnings.push(Warning::new(
                    "agent.script_env_missing",
                    &spec.name,
                    format!(
                        "`{path}` reads {} in its `# env:` header, which is in neither `secrets` nor `env`, so the run would not pass it to the script.",
                        missing.join(", ")
                    ),
                ));
            }
        }
    }
    warnings
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/scripts.rs"]
mod tests;
