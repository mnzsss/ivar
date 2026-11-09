//! Mechanical checks on a feature's task packets, run before a reviewer reads
//! them.
//!
//! A packet that delegation produced can look finished and still be unusable:
//! a placeholder where code belongs, a Readers section with no `git grep`
//! output, a `Create:` of a file the repo already has, a consumer that never
//! waits for the producer of the interface it uses. Every one of those is
//! decidable from the text and the repo's file tree, so none of them should
//! cost a reviewer's attention.
//!
//! The checks read text and ask [`RepoFiles`] one question — how big is this
//! file, if it exists — so the rules stay testable without a repo on disk.

mod deps;
mod packet;

use crate::error::Failure;

/// The size a file may reach before the commit hook refuses it. A file at or
/// above this cannot take a new test.
pub const HOOK_LIMIT_BYTES: u64 = 20_000;

/// A placeholder stands where a packet owes real content.
pub const PLACEHOLDER: &str = "plan.lint.placeholder";
/// The `<NNN>` marker a template leaves behind when nobody filled it in.
pub const NUMBER_PLACEHOLDER: &str = "plan.lint.number_placeholder";
/// The packet has no Readers section.
pub const READERS_MISSING: &str = "plan.lint.readers_missing";
/// The Readers section holds no fenced block, so no real command output.
pub const READERS_WITHOUT_OUTPUT: &str = "plan.lint.readers_without_output";
/// A `**Sketch:**` marker that does not say why literal code is wrong there.
pub const SKETCH_WITHOUT_REASON: &str = "plan.lint.sketch_without_reason";
/// `Modify:` names a file that is not in the repo.
pub const MODIFY_MISSING: &str = "plan.lint.modify_missing";
/// `Create:` names a file the repo already has.
pub const CREATE_EXISTS: &str = "plan.lint.create_exists";
/// A new test lands in a file at or above [`HOOK_LIMIT_BYTES`].
pub const TEST_IN_OVERSIZED_FILE: &str = "plan.lint.test_in_oversized_file";
/// A consumer does not (transitively) wait for the producer of an interface.
pub const MISSING_DEPENDENCY: &str = "plan.lint.missing_dependency";

/// What the repo's file tree can say about a path a packet names.
pub trait RepoFiles {
    /// The file's size in bytes, or `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] when the tree cannot be read.
    fn len(&self, path: &str) -> Result<Option<u64>, Failure>;
}

/// One task packet: its file name (`01-pin-scaffold.md`) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketSource {
    pub file: String,
    pub text: String,
}

/// One problem a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The packet file, or `plan.md` for a problem no packet owns.
    pub file: String,
    /// 1-based line, or `0` when the problem is about the file as a whole.
    pub line: usize,
    /// Stable identifier, one of the constants in this module.
    pub code: &'static str,
    /// One sentence naming the problem.
    pub what: String,
}

impl Finding {
    /// `file`, or `file:line` when the finding points at a line.
    #[must_use]
    pub fn subject(&self) -> String {
        if self.line == 0 {
            self.file.clone()
        } else {
            format!("{}:{}", self.file, self.line)
        }
    }
}

/// Every problem in `packets`, in packet order, then the dependency problems
/// `plan` implies.
///
/// `files` is `None` when no declared repo is on disk: there is nothing to
/// resolve a path against, so the file rules are skipped rather than guessed.
///
/// # Errors
///
/// Returns the [`Failure`] `files` reports when it cannot read the tree.
pub fn lint(
    plan: &str,
    packets: &[PacketSource],
    files: Option<&dyn RepoFiles>,
) -> Result<Vec<Finding>, Failure> {
    let created = packet::created_paths_of(packets);
    let mut findings = Vec::new();
    for source in packets {
        findings.extend(packet::check(source, files, &created)?);
    }
    findings.extend(deps::check(plan, packets));
    Ok(findings)
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/plan_lint.rs"]
mod tests;
