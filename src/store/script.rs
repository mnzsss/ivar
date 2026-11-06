//! Reading the hall's scripts folders — `.ivar/scripts/` and
//! `.ivar/scripts-local/`.
//!
//! **Read-only by design.** Nothing here writes, chmods, moves or deletes a
//! script; the folders belong to the people (and agents) who author scripts
//! there. The format and its validation are pure and live in
//! [`crate::domain::script`]; this module only walks the two folders and hands
//! each file to the parser.
//!
//! # Contract
//!
//! - [`scan`] returns one [`ScriptFile`] per regular file directly inside a
//!   folder. A folder that does not exist contributes nothing.
//! - Dotfiles (`.gitkeep`, `.DS_Store`) and subdirectories are not scripts and
//!   are skipped.
//! - A file that does not satisfy the format is still returned, with its
//!   [`ScriptProblem`], so `doctor` can name it and the brief can omit it.
//! - The order is the one every surface wants: personal scripts first, then
//!   team, alphabetical by name within each group. The same name in both
//!   folders is returned twice.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::script::{self, Script, ScriptProblem, ScriptScope};
use crate::infra::fs;
use crate::store::layout::Layout;

/// One file found in a scripts folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFile {
    /// Which folder it lives in.
    pub scope: ScriptScope,
    /// Its file name, e.g. `deploy.sh`.
    pub file_name: String,
    /// Its absolute path.
    pub path: Utf8PathBuf,
    /// The parsed script, or why the file is not one.
    pub parsed: Result<Script, ScriptProblem>,
}

impl ScriptFile {
    /// The name the script goes by: its stem, or the whole file name when the
    /// file is not a valid script.
    #[must_use]
    pub fn name(&self) -> &str {
        match &self.parsed {
            Ok(script) => &script.name,
            Err(_) => stem(&self.file_name),
        }
    }
}

fn stem(file_name: &str) -> &str {
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
}

/// Every file in both scripts folders, in display order.
///
/// # Errors
///
/// [`fs::Error`] when a folder exists but cannot be listed.
pub fn scan(layout: &Layout) -> Result<Vec<ScriptFile>, fs::Error> {
    let mut files = Vec::new();
    for (dir, scope) in [
        (layout.hall_scripts_local(), ScriptScope::Personal),
        (layout.hall_scripts(), ScriptScope::Team),
    ] {
        read_folder(&dir, scope, &mut files)?;
    }
    files.sort_by(|a, b| (a.scope, a.name(), &a.file_name).cmp(&(b.scope, b.name(), &b.file_name)));
    Ok(files)
}

fn read_folder(
    dir: &Utf8Path,
    scope: ScriptScope,
    files: &mut Vec<ScriptFile>,
) -> Result<(), fs::Error> {
    if !fs::is_dir(dir)? {
        return Ok(());
    }
    for path in fs::read_dir(dir)? {
        let Some(file_name) = path.file_name().map(str::to_owned) else {
            continue;
        };
        if file_name.starts_with('.') || !fs::is_file(&path)? {
            continue;
        }
        let parsed = match script::classify(&file_name) {
            // Reject on the name before reading: a stray `notes.txt` is never opened.
            Err(problem) => Err(problem),
            Ok(_) => match fs::read_text(&path) {
                Ok(Some(source)) => script::parse(&file_name, &source),
                Ok(None) | Err(_) => Err(ScriptProblem::Unreadable),
            },
        };
        files.push(ScriptFile {
            scope,
            file_name,
            path,
            parsed,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/store/script.rs"]
mod tests;
