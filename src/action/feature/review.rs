//! `ivar feature review <name>` — open a feature in VSCode.
//!
//! Writes `<hall>/<feature>.code-workspace`: a VSCode multi-root workspace with
//! one folder per repo in the hall. A repo the feature has **promoted** opens
//! its feature-branch worktree (where the feature's work actually lives); every
//! other repo opens its default-branch worktree (read-only context). Folder
//! paths are relative to the workspace file, which lives at the hall root —
//! VSCode resolves them against the file, so the workspace survives being
//! moved around with the hall.
//!
//! The file is written canonically (sorted keys, atomic write) through
//! `infra::json`, so re-running review never diffs a rewritten workspace
//! against itself.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::json;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar feature review` needs.
#[derive(Debug, Clone)]
pub struct ReviewInput {
    /// The feature's name.
    pub name: String,
}

/// What `ivar feature review` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the workspace was written for.
    pub feature: FeatureName,
    /// The workspace file that was written.
    pub workspace: Utf8PathBuf,
    /// The worktree folders the workspace opens, in repo name order.
    pub folders: Vec<Utf8PathBuf>,
}

impl WriteHuman for ReviewOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Wrote VSCode workspace for `{}` to {}",
            self.feature, self.workspace
        )?;
        for folder in &self.folders {
            writeln!(w, "  {folder}")?;
        }
        Ok(())
    }
}

/// The VSCode multi-root workspace document — `{"folders": [{"path": …}]}`.
/// This is the shape VSCode's `code <feature>.code-workspace` opens.
#[derive(Debug, Serialize)]
struct Workspace {
    folders: Vec<WorkspaceFolder>,
}

/// One folder entry in a [`Workspace`]. `path` is relative to the workspace
/// file, which is what VSCode resolves it against.
#[derive(Debug, Serialize)]
struct WorkspaceFolder {
    path: Utf8PathBuf,
}

/// Write the VSCode workspace for `input.name`.
///
/// Blocked when the feature does not exist. Failed when the workspace cannot
/// be written — nothing else is touched, so a retry is safe.
pub fn review(ctx: &Ctx, input: ReviewInput) -> Outcome<ReviewOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(input.name)?;

    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    let mut repos: Vec<_> = manifest.repos().iter().collect();
    repos.sort_by(|a, b| a.name().cmp(b.name()));

    let mut folders = Vec::new();
    for repo in repos {
        // Promoted repos open their feature-branch worktree — where the
        // feature's work lives; everyone else opens the default branch.
        let worktree = if feature.is_promoted(repo.name()) {
            layout.repo_worktree(repo.name(), &feature.branch)
        } else {
            layout.repo_worktree(repo.name(), repo.default_branch())
        };
        folders.push(worktree);
    }

    // Relative to the hall root — the workspace file's own directory. VSCode
    // resolves each folder path against the file, so the workspace survives
    // being moved around with the hall.
    let workspace = Workspace {
        folders: folders
            .iter()
            .map(|path| WorkspaceFolder {
                path: path
                    .strip_prefix(layout.root())
                    .unwrap_or_else(|_| path.as_path())
                    .to_path_buf(),
            })
            .collect(),
    };

    let workspace_path = layout.workspace_file(&name);
    json::write_canonical(&workspace_path, &workspace)?;

    Ok(Report::new(ReviewOutcome {
        root: layout.root().to_path_buf(),
        feature: name,
        workspace: workspace_path,
        folders,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/review.rs"]
mod tests;
