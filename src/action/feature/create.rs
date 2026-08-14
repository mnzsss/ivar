//! `ivar feature create <name>` — start a feature.
//!
//! A feature is a branch name plus the (initially empty) set of repos
//! promoted onto it. Creating it records that in `features/<name>/` and
//! nothing else: no repo is touched until `promote` says so, and no worktree
//! appears until a repo is promoted onto the branch.
//!
//! A **subfeature** is created with `--parent <feature>`: the child's `base`
//! is derived from the immediate parent's branch (never from `--base`, which
//! conflicts), and only the child-side `parent` fact is persisted — children
//! are derived by scanning, never stored. `--via`/`--strategy` persist the
//! feature's own integration-policy override; omitted fields stay
//! inheritable.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, IntegrationOverride, IntegrationStrategy, IntegrationVia};
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use super::relations;
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
    /// stands in, per [`crate::domain::feature::effective_base`]. Conflicts
    /// with `parent`.
    pub base: Option<String>,
    /// The parent feature's name, unvalidated. `Some` makes this a child:
    /// the base is derived from the parent's branch, and `base` must be
    /// `None`.
    pub parent: Option<String>,
    /// The feature's via override — `pr` or `local`, unvalidated.
    pub via: Option<String>,
    /// The feature's strategy override — `squash`, `merge`, or `rebase`,
    /// unvalidated.
    pub strategy: Option<String>,
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
    /// The branch new promotions will start from, if declared — the parent's
    /// branch for a subfeature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<BranchName>,
    /// The parent, for a subfeature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<FeatureName>,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match (&self.parent, &self.base) {
            (Some(parent), Some(base)) => writeln!(
                w,
                "Created subfeature `{}` of `{parent}` (branch: {}, base: {base}) in {}",
                self.name, self.branch, self.root
            ),
            (Some(parent), None) => writeln!(
                w,
                "Created subfeature `{}` of `{parent}` (branch: {}) in {}",
                self.name, self.branch, self.root
            ),
            (None, Some(base)) => writeln!(
                w,
                "Created feature `{}` (branch: {}, base: {base}) in {}",
                self.name, self.branch, self.root
            ),
            (None, None) => writeln!(
                w,
                "Created feature `{}` (branch: {}) in {}",
                self.name, self.branch, self.root
            ),
        }
    }
}

/// Create a feature named `input.name`, on `input.branch` or on a branch of
/// the same name, under `input.parent` when one is named.
///
/// A [`FeatureName`] is one path segment; a [`BranchName`] is not. So
/// `<name>` → branch `<name>` covers the ordinary case, and `--branch` covers
/// the one it cannot spell: adopting `feat/login`, which is a perfectly good
/// branch and an impossible feature name.
///
/// Refuses when a feature with that name already exists — a second `create`
/// would overwrite promotions that a teammate is already working against.
/// A subfeature additionally requires its parent to exist, and refuses
/// `--base` alongside `--parent` (the base is the parent's branch).
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name.clone())?;
    let branch = BranchName::new(input.branch.unwrap_or(input.name))?;

    // A child's base is always derived from its immediate parent's branch;
    // an explicit base would silently disagree with the lineage.
    if input.base.is_some() && input.parent.is_some() {
        return Err(Failure::blocked(
            "feature.create_parent_and_base_conflict",
            format!("feature `{name}` cannot be created with both `--parent` and `--base`"),
        )
        .expected("`--parent` (which derives the base from the parent's branch) or `--base`, not both")
        .actual("both were provided")
        .fix(FixAction::safe(
            "feature.pick_one_base_source",
            "Use `--parent <feature>` for a subfeature, or `--base <branch>` for an explicit base — not both.",
        )));
    }

    let (base, parent) = match input.parent.clone() {
        Some(raw_parent) => {
            let parent_name = FeatureName::new(raw_parent)?;
            let parent_feature = relations::read_feature(&layout, &parent_name)?;
            (Some(parent_feature.branch.clone()), Some(parent_name))
        }
        None => (input.base.map(BranchName::new).transpose()?, None),
    };

    // The feature's own integration-policy override, parsed independently per
    // field — an omitted field stays inheritable.
    let integration = IntegrationOverride {
        via: input.via.map(|raw| IntegrationVia::parse(&raw)).transpose()?,
        strategy: input
            .strategy
            .map(|raw| IntegrationStrategy::parse(&raw))
            .transpose()?,
    };

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
    feature.parent = parent.clone();
    feature.integration = integration;
    feature.write(&layout)?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        name,
        branch,
        base,
        parent,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/create.rs"]
mod tests;
