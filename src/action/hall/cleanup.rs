//! `ivar cleanup` — remove work left behind, asking before anything is
//! deleted.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::Ctx;
use super::{ask, discover_hall, read_manifest};

/// What `ivar cleanup` would remove, or removed.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// Everything removed.
    pub removed: Vec<String>,
    /// Everything declined (the user said no, or the run was not a tty).
    pub kept: Vec<String>,
}

impl WriteHuman for CleanupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.removed.is_empty() && self.kept.is_empty() {
            writeln!(w, "Nothing to clean up in {}.", self.root)?;
            return Ok(());
        }
        for path in &self.removed {
            writeln!(w, "  removed {path}")?;
        }
        for path in &self.kept {
            writeln!(w, "  kept    {path}")?;
        }
        Ok(())
    }
}

/// Remove stale state, asking before anything is deleted.
///
/// This is the one verb that can destroy work, so it is **interactive by
/// design** (ARCHITECTURE.md: `cleanup` is where removal lives, and it will
/// ask) and deliberately has no `--force` / `--dry-run` automation flags.
/// On a non-tty run it lists what *would* be removed and keeps everything —
/// a script can never delete through `ivar cleanup`.
///
/// What it removes today: bare clones of repos no longer in the manifest.
/// (Worktree removal is where uncommitted work lives; that stays a manual
/// `git worktree remove` until a later slice.)
pub fn cleanup(ctx: &Ctx) -> Outcome<CleanupOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let mut removed = Vec::new();
    let mut kept = Vec::new();

    let repos_dir = layout.repos_dir();
    if fs::is_dir(&repos_dir)? {
        for entry in fs::read_dir(&repos_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            // A repo still in the manifest is not stale.
            if manifest
                .repos()
                .iter()
                .any(|repo| repo.name().as_str() == name)
            {
                continue;
            }
            let repo_dir = repos_dir.join(name);
            if ask_remove(&repo_dir)? {
                fs::remove_path(&repo_dir)?;
                removed.push(repo_dir.to_string());
            } else {
                kept.push(repo_dir.to_string());
            }
        }
    }

    Ok(Report::new(CleanupOutcome {
        root: layout.root().to_path_buf(),
        removed,
        kept,
    }))
}

/// Ask before removing `path`. Returns `true` to remove, `false` to keep.
///
/// Non-tty runs answer `false` — cleanup must never delete without a human
/// looking at the question.
fn ask_remove(path: &Utf8Path) -> Result<bool, Failure> {
    ask(
        &format!("Remove `{path}`?"),
        "cleanup.write_prompt",
        "cleanup.read_answer",
        None,
    )
}
