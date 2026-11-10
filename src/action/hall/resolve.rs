//! The fixed rules that settle a hall's rebase conflicts without a model.
//!
//! [`resolve_conflicts`] is the one place a conflicted hall rebase is
//! settled. Today it applies two mechanical rules; a path they do not cover is
//! reported back as [`Resolution::Unresolved`] and the caller abandons the
//! rebase. Anything smarter — settling a conflict in prose — belongs *here*,
//! behind the same signature, so the sync's control flow never learns who did
//! the resolving.
//!
//! # The rules
//!
//! During a rebase *ours* is the branch being rebased onto — the remote — and
//! *theirs* is the local commit being replayed.
//!
//! - **`.ivar/skills/state.json`**: the remote's version stays. The file is a
//!   record of what the skill sync last materialised; the sync that called
//!   this runs the skill plan again afterwards, which rewrites it from what is
//!   actually on disk, so neither side's copy is worth merging by hand.
//! - **Dated run files** — `reports/**/YYYY-MM-DD*` and
//!   `brand/metricas/YYYY-MM-DD*`: the output of a scheduled run on one
//!   machine. Two machines running the same day produce the same name. Equal
//!   content keeps one copy; different content keeps the remote's at the
//!   original name and saves the local one beside it as
//!   `<name>-<hostname>.<ext>`, so neither run's output is lost.

use camino::{Utf8Path, Utf8PathBuf};

use crate::git::{Conflict, Error, Git, Side};
use crate::infra::{fs, proc};

/// The skill sync's record of what it materialised, hall-relative.
pub(crate) const SKILLS_STATE: &str = ".ivar/skills/state.json";

/// How a set of conflicts ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Every conflicted path is settled and staged; the rebase can continue.
    Resolved,
    /// These paths are not covered by any rule. Nothing was changed.
    Unresolved(Vec<Utf8PathBuf>),
}

/// Settle `conflicts` in the hall at `root` with the fixed rules, staging
/// what it settles.
///
/// All or nothing: when any path is outside the rules, nothing is touched and
/// those paths come back in [`Resolution::Unresolved`], so the caller can
/// abort a rebase that is still exactly as git left it.
///
/// # Errors
///
/// Returns [`Error`] if git or the filesystem refuses part-way. The rebase is
/// then in an unknown state and the caller must abort it.
pub(crate) fn resolve_conflicts(
    git: &impl Git,
    root: &Utf8Path,
    conflicts: &[Conflict],
) -> Result<Resolution, Error> {
    let unresolved: Vec<Utf8PathBuf> = conflicts
        .iter()
        .filter(|conflict| rule_for(conflict).is_none())
        .map(|conflict| conflict.path.clone())
        .collect();
    if !unresolved.is_empty() {
        return Ok(Resolution::Unresolved(unresolved));
    }

    let mut staged: Vec<Utf8PathBuf> = Vec::new();
    for conflict in conflicts {
        match rule_for(conflict) {
            Some(Rule::KeepRemote) => git.take_side(root, &conflict.path, Side::Ours)?,
            Some(Rule::KeepBoth) => keep_both(git, root, conflict, &mut staged)?,
            None => continue,
        }
        staged.push(conflict.path.clone());
    }
    git.stage_paths(root, &staged)?;
    Ok(Resolution::Resolved)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    KeepRemote,
    KeepBoth,
}

/// The rule that covers `conflict`, if any. A rule needs the remote's version
/// to exist, and the dated-file rule needs the local one too: a path one side
/// deleted is a decision, not a collision.
fn rule_for(conflict: &Conflict) -> Option<Rule> {
    if conflict.path.as_str() == SKILLS_STATE {
        return conflict.ours.is_some().then_some(Rule::KeepRemote);
    }
    (is_dated_run_file(&conflict.path) && conflict.ours.is_some() && conflict.theirs.is_some())
        .then_some(Rule::KeepBoth)
}

/// Whether `path` is `reports/**/YYYY-MM-DD*` or `brand/metricas/YYYY-MM-DD*`.
fn is_dated_run_file(path: &Utf8Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    if !starts_with_date(name) {
        return false;
    }
    let under_reports = path.starts_with("reports") && path.components().count() >= 2;
    under_reports || path.parent() == Some(Utf8Path::new("brand/metricas"))
}

/// Whether `name` begins with four digits, a dash, two digits, a dash, two
/// digits.
fn starts_with_date(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 10
        && bytes.iter().take(10).enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

/// Keep the remote's version at `conflict.path`; when the two differ, save the
/// local one beside it first. The saved copy is staged along with the path.
fn keep_both(
    git: &impl Git,
    root: &Utf8Path,
    conflict: &Conflict,
    staged: &mut Vec<Utf8PathBuf>,
) -> Result<(), Error> {
    if conflict.ours != conflict.theirs {
        git.take_side(root, &conflict.path, Side::Theirs)?;
        let local = root.join(&conflict.path);
        let saved = free_name(root, &conflict.path)?;
        let bytes = fs::read_bytes(&local)?.unwrap_or_default();
        fs::write_bytes(&root.join(&saved), &bytes)?;
        staged.push(saved);
    }
    git.take_side(root, &conflict.path, Side::Ours)
}

/// `<stem>-<hostname>.<ext>` beside `path`, or with `-2`, `-3`… added when an
/// earlier conflict already took that name.
fn free_name(root: &Utf8Path, path: &Utf8Path) -> Result<Utf8PathBuf, Error> {
    let host = hostname();
    let stem = path.file_stem().unwrap_or("run");
    let dir = path.parent().unwrap_or(Utf8Path::new(""));
    let mut attempt = 1_u32;
    loop {
        let tag = if attempt == 1 {
            host.clone()
        } else {
            format!("{host}-{attempt}")
        };
        let name = match path.extension() {
            Some(extension) => format!("{stem}-{tag}.{extension}"),
            None => format!("{stem}-{tag}"),
        };
        let candidate = dir.join(name);
        if !fs::exists(&root.join(&candidate))? {
            return Ok(candidate);
        }
        attempt += 1;
    }
}

/// This machine's name, reduced to what is safe in a file name.
fn hostname() -> String {
    let raw = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_text(Utf8Path::new("/etc/hostname")).ok().flatten())
        .or_else(|| {
            proc::capture(&proc::Command::new("hostname"))
                .ok()
                .filter(proc::Output::success)
                .map(|output| output.stdout)
        })
        .unwrap_or_default();
    let clean: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches(|c| c == '-' || c == '.');
    if clean.is_empty() {
        "local".to_owned()
    } else {
        clean.to_owned()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/hall/resolve.rs"]
mod tests;
