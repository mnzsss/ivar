//! `ivar feature reparent <child> --parent <new-parent>` — move a still
//! pristine child under a different parent.
//!
//! Reparenting is the **one** allowed lineage transition, and it is allowed
//! only while the child is pristine: no promotions, no receipts, no plan,
//! execution, or session state, no close record, and no descendants. Once any
//! work exists, the parent (and the derived base, and the feature-wide policy)
//! are immutable — the explicit reparent command is the only way they ever
//! change, and it refuses before work starts.
//!
//! The whole mutation is one canonical `feature.json` write: `child.parent =
//! new_parent.name` and `child.base = new_parent.branch` land together or not
//! at all, never as separate writes that could disagree.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use super::lifecycle::read_close;
use super::relations;
use crate::action::Ctx;
use crate::store::layout::Layout;

/// What `ivar feature reparent` needs.
#[derive(Debug, Clone)]
pub struct ReparentInput {
    /// The child feature to move.
    pub child: String,
    /// The new parent feature.
    pub parent: String,
}

/// What `ivar feature reparent` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReparentOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The child that moved.
    pub child: FeatureName,
    /// The parent the child moved *from*, if it had one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_parent: Option<FeatureName>,
    /// The parent the child moved under.
    pub new_parent: FeatureName,
    /// The child's derived base — the new parent's branch.
    pub base: BranchName,
}

impl WriteHuman for ReparentOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match &self.old_parent {
            Some(old) => writeln!(
                w,
                "Moved subfeature `{}` from under `{old}` to under `{}` (base: {}) in {}",
                self.child, self.new_parent, self.base, self.root
            ),
            None => writeln!(
                w,
                "Made `{}` a subfeature of `{}` (base: {}) in {}",
                self.child, self.new_parent, self.base, self.root
            ),
        }
    }
}

/// Move `input.child` under `input.parent`, in one atomic record write.
///
/// Refuses — before any mutation — when the child or the new parent does not
/// exist, when the new parent is the child itself or lies below it (either
/// would cycle the tree), or when any work fact exists: a promotion, a
/// receipt, a plan/execution/session entry, a close record, or a descendant.
pub fn reparent(ctx: &Ctx, input: ReparentInput) -> Outcome<ReparentOutcome> {
    let layout = discover_hall(ctx)?;
    let child_name = FeatureName::new(input.child)?;
    let new_parent_name = FeatureName::new(input.parent)?;

    // Read the whole tree first: a corrupt tree (missing parent or cycle)
    // refuses before this verb considers mutating anything.
    relations::read_all(&layout)?;
    let child = relations::read_feature(&layout, &child_name)?;

    let old_parent = child.parent.clone();
    if child_name == new_parent_name {
        return Err(Failure::blocked(
            "feature.reparent_self_parent",
            format!("cannot reparent `{child_name}` under itself"),
        )
        .expected("a different parent than the child itself")
        .actual("the child was named as its own parent")
        .fix(FixAction::safe(
            "feature.reparent_pick_another",
            "Pick a parent that is not the child itself.",
        )));
    }
    if child.parent.as_ref() == Some(&new_parent_name) {
        return Err(Failure::blocked(
            "feature.reparent_same_parent",
            format!(
                "feature `{child_name}` is already a subfeature of `{new_parent_name}`"
            ),
        )
        .expected("a different parent than the child already has")
        .actual("the child's current parent was named again")
        .fix(FixAction::safe(
            "feature.reparent_pick_another",
            "Pick a parent the child does not already sit under.",
        )));
    }

    // The new parent must exist — a lineage can only point at a real feature.
    let new_parent = relations::read_feature(&layout, &new_parent_name).map_err(|_| {
        Failure::blocked(
            "feature.reparent_parent_not_found",
            format!("cannot reparent `{child_name}`: parent `{new_parent_name}` does not exist"),
        )
        .expected("the new parent to be an existing feature")
        .actual(format!("`{new_parent_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_parent_first",
            format!("Create `{new_parent_name}` first, or pick a parent that exists."),
        ))
    })?;

    // A target below the child would cycle the tree: walking down from the
    // child must never reach the proposed parent.
    let descendants = relations::descendants(&layout, &child_name)?;
    if descendants
        .iter()
        .any(|descendant| descendant.name == new_parent_name)
    {
        return Err(Failure::blocked(
            "feature.reparent_cycle",
            format!(
                "cannot reparent `{child_name}` under `{new_parent_name}`: the proposed parent is one of the child's own descendants"
            ),
        )
        .expected("a new parent that is not the child or one of its descendants")
        .actual(format!("`{new_parent_name}` lies below `{child_name}`"))
        .fix(FixAction::safe(
            "feature.reparent_pick_ancestor",
            "Pick a parent outside the child's own subtree.",
        )));
    }

    // Pristine gate: any work fact freezes the lineage. All checks happen
    // before the single write below.
    if work_started(&layout, &child, &descendants)? {
        return Err(Failure::blocked(
            "feature.reparent_work_started",
            format!("cannot reparent `{child_name}`: work has started"),
        )
        .expected(
            "a pristine child — no promotions, receipts, plan/execution/session state, close record, or descendants",
        )
        .actual("at least one work fact exists")
        .fix(FixAction::safe(
            "feature.reparent_before_work",
            "Reparent a freshly created child before any work starts; afterwards the parent is immutable.",
        )));
    }

    // Exactly one persisted mutation: parent and derived base together.
    let mut updated = child;
    updated.parent = Some(new_parent_name.clone());
    updated.base = Some(new_parent.branch.clone());
    updated.write(&layout)?;

    Ok(Report::new(ReparentOutcome {
        root: layout.root().to_path_buf(),
        child: child_name,
        old_parent,
        new_parent: new_parent_name,
        base: new_parent.branch.clone(),
    }))
}

/// Whether any work fact exists on the child: a promotion, a receipt, plan/
/// execution/session entries, a close record, or a descendant.
fn work_started(
    layout: &Layout,
    child: &Feature,
    descendants: &[Feature],
) -> Result<bool, Failure> {
    if !child.promotions.is_empty() {
        return Ok(true);
    }
    if !descendants.is_empty() {
        return Ok(true);
    }
    if read_close(layout, &child.name)?.is_some() {
        return Ok(true);
    }
    for dir in [
        layout.plan_dir(&child.name),
        layout.execution_dir(&child.name),
        layout.feature_sessions_dir(&child.name),
    ]
    .into_iter()
    {
        if fs::is_dir(&dir)? && !fs::read_dir(&dir)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/reparent.rs"]
mod tests;
