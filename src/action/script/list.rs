//! `ivar script list` — every script in both folders.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::script::{ScriptKind, ScriptScope};
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::script::{ScriptFile, scan};

/// The execution-approval state of a script. Fixed to `unknown` until
/// approvals exist: the field is reserved so consumers can rely on it.
const APPROVAL_UNKNOWN: &str = "unknown";

/// One valid script.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptSummary {
    /// The file stem.
    pub name: String,
    /// `personal` (`.ivar/scripts-local/`) or `team` (`.ivar/scripts/`).
    pub scope: ScriptScope,
    /// The interpreter its extension selects.
    pub kind: ScriptKind,
    /// The path relative to the hall root.
    pub path: String,
    /// The header's one-line description.
    pub description: String,
    /// The header's usage line.
    pub usage: String,
    /// The environment variables the header names.
    pub env: Vec<String>,
    /// Always `unknown` for now: execution approvals are not recorded yet.
    pub approval: &'static str,
}

/// A file in a scripts folder that is not a valid script.
#[derive(Debug, Clone, Serialize)]
pub struct InvalidScript {
    /// Which folder it lives in.
    pub scope: ScriptScope,
    /// The path relative to the hall root.
    pub path: String,
    /// A stable code for what is wrong.
    pub code: &'static str,
    /// What is wrong, in one sentence.
    pub what: String,
}

/// What `ivar script list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root.
    pub root: camino::Utf8PathBuf,
    /// Valid scripts: personal first, then team, alphabetical within each.
    pub scripts: Vec<ScriptSummary>,
    /// Files that are not valid scripts; `ivar doctor` names the same ones.
    pub invalid: Vec<InvalidScript>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.scripts.is_empty() && self.invalid.is_empty() {
            return writeln!(w, "No scripts in {}.", self.root);
        }
        if !self.scripts.is_empty() {
            writeln!(w, "Scripts in {}:", self.root)?;
            let mut table = crate::infra::table::new(&[
                "NAME",
                "SCOPE",
                "DESCRIPTION",
                "USAGE",
                "ENV",
                "APPROVAL",
            ]);
            for script in &self.scripts {
                table.add_row(vec![
                    script.name.as_str(),
                    script.scope.as_str(),
                    &script.description,
                    &script.usage,
                    &script.env.join(", "),
                    script.approval,
                ]);
            }
            crate::infra::table::write(w, &table)?;
        }
        if !self.invalid.is_empty() {
            writeln!(w, "Not valid scripts:")?;
            for invalid in &self.invalid {
                writeln!(w, "  - {} — {}", invalid.path, invalid.what)?;
            }
        }
        Ok(())
    }
}

/// List the scripts in both folders. Read-only.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let mut scripts = Vec::new();
    let mut invalid = Vec::new();
    for file in scan(&layout)? {
        match &file.parsed {
            Ok(script) => scripts.push(ScriptSummary {
                name: script.name.clone(),
                scope: file.scope,
                kind: script.kind,
                path: super::hall_relative(&layout, &file),
                description: script.description.clone(),
                usage: script.usage.clone(),
                env: script.env.clone(),
                approval: APPROVAL_UNKNOWN,
            }),
            Err(problem) => invalid.push(invalid_script(&layout, &file, problem)),
        }
    }
    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        scripts,
        invalid,
    }))
}

fn invalid_script(
    layout: &crate::store::layout::Layout,
    file: &ScriptFile,
    problem: &crate::domain::script::ScriptProblem,
) -> InvalidScript {
    InvalidScript {
        scope: file.scope,
        path: super::hall_relative(layout, file),
        code: problem.code(),
        what: problem.to_string(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/script/list.rs"]
mod tests;
