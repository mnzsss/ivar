//! `ivar feature execute reconcile <feature> --workstream <id> --description
//! "..."` — fold a workstream's code divergence into the board's journal.
//!
//! # What it does
//!
//! When an executor's implementation drifts from what the plan's Operations
//! prescribed, the divergence is recorded here rather than silently
//! forgotten: this verb reads the board's journal for the workstream's prior
//! entries, captures the uncommitted `git diff` across the feature's promoted
//! worktrees, and appends a `reconcile` journal entry joining the caller's
//! description with that diff.
//!
//! **The plan is never rewritten.** Folding the divergence back into
//! `plan.md`'s Operations requires human acceptance of the changed sections
//! first — v1 records only, and the plan stays exactly as it was. Rewriting
//! it is a separate, future step; see the Valhalla definition of
//! OP-RECONCILE ("requires user acceptance before writing").

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, Feature, JournalEntry};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::{find_workstream, require_board};
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute reconcile` needs.
#[derive(Debug, Clone)]
pub struct ReconcileInput {
    /// The feature whose board records the divergence.
    pub feature: String,
    /// The workstream the divergence belongs to.
    pub workstream: String,
    /// The executor's own description of what changed and why.
    pub description: String,
}

/// What `ivar feature execute reconcile` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The workstream the divergence was recorded for.
    pub workstream: String,
    /// The messages of earlier journal entries for this workstream — the
    /// context the divergence is folded into.
    pub prior_deviations: Vec<String>,
    /// The uncommitted `git diff` captured across the feature's worktrees.
    pub diff: String,
    /// The board after the reconciliation.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReconcileOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Recorded reconciliation for `{}` workstream `{}` at {}",
            self.feature, self.workstream, self.board_path
        )
    }
}

/// Record a divergence for `input.workstream` in the board's journal.
///
/// Blocked when the feature has no board or the workstream is unknown. The
/// plan is never touched — only the journal grows, and the board is persisted
/// with the new entry.
pub fn reconcile(ctx: &Ctx, input: ReconcileInput) -> Outcome<ReconcileOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = require_board(&layout, &feature)?;
    let board_path = feature::board_path(&layout, &feature);
    find_workstream(&board, &feature, &input.workstream)?;

    let prior_deviations: Vec<String> = board
        .journal
        .iter()
        .filter(|entry| entry.workstream == input.workstream)
        .map(|entry| entry.message.clone())
        .collect();
    let diff = feature_diff(&layout, &feature)?;

    let message = if diff.is_empty() {
        format!(
            "Reconciled divergence: {} (no uncommitted diff in the feature worktrees)",
            input.description
        )
    } else {
        format!("Reconciled divergence: {}\n{diff}", input.description)
    };
    board.push_journal(JournalEntry::new(&input.workstream, "reconcile", message));
    board.write(&layout, &feature)?;

    Ok(Report::new(ReconcileOutcome {
        root: layout.root().to_path_buf(),
        feature,
        workstream: input.workstream,
        prior_deviations,
        diff,
        board,
        board_path,
    }))
}

/// The uncommitted divergence across the feature's promoted worktrees: for
/// every promoted repo whose worktree exists, `git diff HEAD`, prefixed with
/// the repo name. Empty when the feature has no promoted worktrees, or none
/// of them diverge. The feature's branch comes from its promotion record, so
/// this reads the right worktree per repo.
fn feature_diff(layout: &Layout, feature_name: &FeatureName) -> Result<String, Failure> {
    let Some(feature) = Feature::read(layout, feature_name)? else {
        return Ok(String::new());
    };
    let git = git::System;
    let mut parts = Vec::new();
    for repo in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo, &feature.branch);
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let diff = git.diff_worktree(&worktree)?;
        if !diff.is_empty() {
            parts.push(format!("[{repo}]\n{diff}"));
        }
    }
    Ok(parts.join("\n"))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::domain::name::{BranchName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-impl",
                "title": "Implement",
                "operations": ["write-code"],
                "depends_on": [],
                "write_contract": ["src/"]
            }
        ]
    }"#;

    /// A hall with a seeded repo promoted into the feature, a plan, and a
    /// prepared board.
    fn hall_with_promoted_worktree() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        let origin = seeded_repo(&root.join("origins").join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();
        plan_create::create(
            &ctx,
            PlanCreateInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        prepare_action::prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();
        (guard, root)
    }

    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn reconcile_appends_the_divergence_without_rewriting_the_plan() {
        let (_guard, root) = hall_with_promoted_worktree();
        let ctx = Ctx::new(root.clone());

        // The executor diverges: README.md gains an uncommitted line in the
        // feature worktree.
        let worktree = root.join(".ivar/repos/api/checkout");
        let readme = worktree.join("README.md");
        let original = fs::read_text(&readme).unwrap().unwrap();
        fs::write_text(&readme, &format!("{original}diverged\n")).unwrap();
        let plan_before = fs::read_text(&root.join("plans/checkout/plan.md"))
            .unwrap()
            .unwrap();

        let report = reconcile(
            &ctx,
            ReconcileInput {
                feature: "checkout".to_owned(),
                workstream: "ws-impl".to_owned(),
                description: "implemented auth differently".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.workstream, "ws-impl");
        assert!(report.value.diff.contains("diverged"));
        assert!(report.value.prior_deviations.is_empty());

        // The journal records it, and the plan.md is byte-for-byte untouched.
        let on_disk = persisted(&root);
        let entry = on_disk.journal.last().unwrap();
        assert_eq!(entry.kind, "reconcile");
        assert_eq!(entry.workstream, "ws-impl");
        assert!(entry.message.contains("implemented auth differently"));
        assert!(entry.message.contains("diverged"));
        assert_eq!(
            fs::read_text(&root.join("plans/checkout/plan.md"))
                .unwrap()
                .unwrap(),
            plan_before,
            "reconcile must never rewrite the plan"
        );
    }

    #[test]
    fn reconcile_records_a_divergence_without_promoted_worktrees() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();
        plan_create::create(
            &ctx,
            PlanCreateInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        prepare_action::prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();

        let report = reconcile(
            &ctx,
            ReconcileInput {
                feature: "checkout".to_owned(),
                workstream: "ws-impl".to_owned(),
                description: "no repos promoted".to_owned(),
            },
        )
        .unwrap();

        assert!(report.value.diff.is_empty());
        let on_disk = persisted(&root);
        let entry = on_disk.journal.last().unwrap();
        assert_eq!(entry.kind, "reconcile");
        assert!(entry.message.contains("no uncommitted diff"));
    }

    #[test]
    fn reconcile_is_blocked_without_a_board() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();

        let failure = reconcile(
            &ctx,
            ReconcileInput {
                feature: "checkout".to_owned(),
                workstream: "ws-impl".to_owned(),
                description: "nothing".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_missing");
    }

    #[test]
    fn reconcile_is_blocked_for_an_unknown_workstream() {
        let (_guard, root) = hall_with_promoted_worktree();
        let ctx = Ctx::new(root.clone());

        let failure = reconcile(
            &ctx,
            ReconcileInput {
                feature: "checkout".to_owned(),
                workstream: "ws-ghost".to_owned(),
                description: "nothing".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.workstream_not_found");
    }
}
