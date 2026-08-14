//! `ivar feature create <name>` — start a feature.
//!
//! A feature is a branch name plus the (initially empty) set of repos
//! promoted onto it. Creating it records that in `features/<name>/` and
//! nothing else: no repo is touched until `promote` says so, and no worktree
//! appears until a repo is promoted onto the branch.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The feature's name, unvalidated — [`FeatureName`] is this module's job.
    pub name: String,
    /// The branch to use, unvalidated. `None` derives it from the name.
    ///
    /// The two differ when a branch that already exists cannot be spelled as a
    /// feature name: a [`FeatureName`] is one path segment, while `feat/login`
    /// is an ordinary branch. Without this, such a branch is unreachable —
    /// `promote` can adopt it, but no feature could ever name it.
    pub branch: Option<String>,
    /// The branch new promotions should start from, unvalidated. `None`
    /// leaves the feature's base undeclared — each repo's own default branch
    /// stands in, per [`crate::domain::feature::effective_base`].
    pub base: Option<String>,
}

/// What `ivar feature create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature, as created.
    pub name: FeatureName,
    /// The branch every promoted repo's worktree will be checked out on.
    pub branch: BranchName,
    /// The branch new promotions will start from, if declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<BranchName>,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match &self.base {
            Some(base) => writeln!(
                w,
                "Created feature `{}` (branch: {}, base: {base}) in {}",
                self.name, self.branch, self.root
            ),
            None => writeln!(
                w,
                "Created feature `{}` (branch: {}) in {}",
                self.name, self.branch, self.root
            ),
        }
    }
}

/// Create a feature named `input.name`, on `input.branch` or on a branch of
/// the same name.
///
/// A [`FeatureName`] is one path segment; a [`BranchName`] is not. So
/// `<name>` → branch `<name>` covers the ordinary case, and `--branch` covers
/// the one it cannot spell: adopting `feat/login`, which is a perfectly good
/// branch and an impossible feature name.
///
/// Refuses when a feature with that name already exists — a second `create`
/// would overwrite promotions that a teammate is already working against.
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name.clone())?;
    let branch = BranchName::new(input.branch.unwrap_or(input.name))?;
    let base = input.base.map(BranchName::new).transpose()?;

    let dir = layout.feature_dir(&name);
    if fs::is_dir(&dir)? {
        return Err(Failure::blocked(
            "feature.already_exists",
            format!("feature `{name}` already exists"),
        )
        .expected("a feature name that has not been used before")
        .actual(format!("`{}` already has a feature directory", dir))
        .fix(FixAction::safe(
            "feature.use_existing",
            "Use the existing feature, or pick a different name.",
        )));
    }

    let mut feature = Feature::new(name.clone(), branch.clone());
    feature.base = base.clone();
    feature.write(&layout)?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        name,
        branch,
        base,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/create.rs"]
mod tests;
