//! The hall's scripts folders. Read-only, and only ever a warning: a script
//! with a missing or malformed header, or an unsupported extension, is named
//! here, never made an error. `ivar` does not touch the file.
//!
//! Every readable script is also searched for secrets, the same patterns
//! `ivar script approve` refuses, and for lines that name a secret file
//! (`.env`, `agents.env`). The second check is lexical: see
//! [`crate::domain::script_approval::secret_file_references`].

use crate::domain::sanitize::find_secrets;
use crate::domain::script::{ScriptProblem, classify};
use crate::domain::script_approval::secret_file_references;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::script::{ScriptFile, scan};

use super::Diagnosis;

pub(super) fn script_diagnoses(layout: &Layout) -> Vec<Diagnosis> {
    // An unlistable folder is not a script problem; the other checks report
    // a hall they cannot read.
    let Ok(files) = scan(layout) else {
        return Vec::new();
    };
    let mut diagnoses = Vec::new();
    for file in &files {
        if let Err(problem) = &file.parsed {
            diagnoses.push(Diagnosis {
                code: problem.code(),
                what: format!(
                    "`{}`: {problem}",
                    crate::action::script::hall_relative(layout, file)
                ),
                fix: fix(problem).to_owned(),
            });
        }
        diagnoses.extend(content_diagnoses(layout, file));
    }
    diagnoses
}

/// Warnings about what a script's text contains. A file that is not a script
/// by name (`notes.txt`) is never opened, and one that cannot be read as text
/// is already named by its [`ScriptProblem::Unreadable`].
fn content_diagnoses(layout: &Layout, file: &ScriptFile) -> Vec<Diagnosis> {
    if classify(&file.file_name).is_err() {
        return Vec::new();
    }
    let Ok(Some(source)) = fs::read_text(&file.path) else {
        return Vec::new();
    };
    let path = crate::action::script::hall_relative(layout, file);
    let mut diagnoses = Vec::new();
    let secrets = find_secrets(&source);
    if !secrets.is_empty() {
        let at: Vec<String> = secrets
            .iter()
            .map(|found| format!("line {} ({})", found.line, found.pattern))
            .collect();
        diagnoses.push(Diagnosis {
            code: "script.secret_found",
            what: format!("`{path}` matches a secret pattern at {}", at.join(", ")),
            fix: "Remove the secret from the script and read it from an environment variable \
                  named in `# env:`. `ivar script approve` refuses this content."
                .to_owned(),
        });
    }
    let references = secret_file_references(&source);
    if !references.is_empty() {
        let at: Vec<String> = references
            .iter()
            .map(|found| format!("line {} (`{}`)", found.line, found.file))
            .collect();
        diagnoses.push(Diagnosis {
            code: "script.secret_file",
            what: format!("`{path}` references a secret file at {}", at.join(", ")),
            fix: "Pass the values in through environment variables named in `# env:` instead \
                  of reading a secrets file."
                .to_owned(),
        });
    }
    diagnoses
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
