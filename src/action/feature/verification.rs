//! The ordered verification runner: each manifest repo's `checks` executed as
//! real commands, with a deterministic fingerprint and durable evidence.
//!
//! A repo's `checks` are executable policy, not preview-only text: `ivar`
//! runs them via `bash -lc` in the relevant worktree, in order, stopping at
//! the first failure, and records every result in the integration receipt.
//! The fingerprint of the command list is what makes a receipt stale when the
//! hall's checks change — see [`fingerprint`].
//!
//! This module renders nothing and previews nothing. If a command list was
//! not run, no evidence exists, and a receipt must not claim it does.

use camino::Utf8Path;

use crate::domain::feature::VerificationResult;
use crate::error::Failure;
use crate::infra::{hash, json, proc};

/// One ordered verification pass: the fingerprint of the command list plus
/// the results that actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerificationRun {
    /// [`fingerprint`] of the command list that produced `results`.
    pub command_fingerprint: String,
    /// The results, in execution order. Stops at the first failure, so a
    /// failing run has fewer entries than commands.
    pub results: Vec<VerificationResult>,
}

/// A deterministic fingerprint of an ordered command list — SHA-256 of its
/// canonical JSON. Receipt freshness compares the fingerprint recorded at
/// verification time against this, so editing a repo's checks invalidates old
/// receipts.
pub(crate) fn fingerprint(commands: &[String]) -> Result<String, Failure> {
    let rendered = json::to_canonical_string(&commands.to_vec()).map_err(Failure::from)?;
    Ok(hash::text(&rendered))
}

/// Run `commands` via `bash -lc <command>` in `cwd`, in order, stopping at
/// the first nonzero exit or spawn failure. Each result records the command,
/// its exit code (when it exited with one), and its most useful diagnostic.
pub(crate) fn run(commands: &[String], cwd: &Utf8Path) -> Result<VerificationRun, Failure> {
    let command_fingerprint = fingerprint(commands)?;
    let mut results = Vec::new();

    for command in commands {
        let invocation = proc::Command::new("bash").arg("-lc").arg(command).cwd(cwd);
        let result = match proc::capture(&invocation) {
            Ok(output) => VerificationResult {
                command: command.clone(),
                success: output.success(),
                exit_code: output.code,
                diagnostic: output.diagnostic(),
            },
            // A spawn failure is still a recorded, stopping failure — the
            // diagnostic explains why the command never ran.
            Err(error) => VerificationResult {
                command: command.clone(),
                success: false,
                exit_code: None,
                diagnostic: error.to_string(),
            },
        };
        let success = result.success;
        results.push(result);
        if !success {
            break;
        }
    }

    Ok(VerificationRun {
        command_fingerprint,
        results,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/verification.rs"]
mod tests;
