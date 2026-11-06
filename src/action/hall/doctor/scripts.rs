//! The hall's scripts folders. Read-only, and only ever a warning: a script
//! with a missing or malformed header, or an unsupported extension, is named
//! here, never made an error. `ivar` does not touch the file.

use crate::domain::script::ScriptProblem;
use crate::store::layout::Layout;
use crate::store::script::scan;

use super::Diagnosis;

pub(super) fn script_diagnoses(layout: &Layout) -> Vec<Diagnosis> {
    // An unlistable folder is not a script problem; the other checks report
    // a hall they cannot read.
    let Ok(files) = scan(layout) else {
        return Vec::new();
    };
    files
        .iter()
        .filter_map(|file| {
            let problem = file.parsed.as_ref().err()?;
            Some(Diagnosis {
                code: problem.code(),
                what: format!(
                    "`{}`: {problem}",
                    crate::action::script::hall_relative(layout, file)
                ),
                fix: fix(problem).to_owned(),
            })
        })
        .collect()
}

fn fix(problem: &ScriptProblem) -> &'static str {
    match problem {
        ScriptProblem::UnsupportedExtension => {
            "Rename it to a `.sh` or `.py` script, or move it out of the scripts folder."
        }
        ScriptProblem::BadName(_) => {
            "Rename the file to a kebab-case name, e.g. `deploy-staging.sh`."
        }
        ScriptProblem::MissingDescription | ScriptProblem::MissingUsage => {
            "Add `# description:` and `# usage:` lines in the comment header right after the shebang."
        }
        ScriptProblem::DuplicateField(_) => "Keep one line per header field.",
        ScriptProblem::BadEnvName(_) => {
            "List environment variable names in `# env:`, separated by commas."
        }
        ScriptProblem::Unreadable => "Make the file readable UTF-8 text.",
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/doctor/scripts.rs"]
mod tests;
