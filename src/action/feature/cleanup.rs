//! `ivar feature cleanup` — build the side-effect-free cleanup preview.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::action::feature::delete;
use crate::action::session::lookup as session_lookup;
use crate::domain::feature::{
    BranchDeletion, CleanupApplyOutcome, CleanupBlocker, CleanupDescendant, CleanupDescendantFacts,
    CleanupDescendantOutcome, CleanupFacts, CleanupPreview, CleanupRecord, CleanupRepo,
    CleanupRepoFacts, Feature, ForgeDelivery, WorktreeRemoval, classify_cleanup,
    classify_descendant_cleanup,
};
use crate::domain::name::{FeatureName, RepoName, SessionId};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git, TargetState, WorktreeEntry};
use crate::infra::{fs, hash, json};

use super::super::{discover_hall, read_manifest};
use super::pull_requests::{self, PullRequest};
use super::{base, relations};

#[derive(Debug, Clone)]
pub struct CleanupInput {
    pub feature: String,
    pub preview: bool,
    pub record: Option<Utf8PathBuf>,
    /// The session running cleanup (`$IVAR_SESSION_ID`); never counted as a live-session blocker.
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    pub root: Utf8PathBuf,
    pub preview: CleanupPreview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_outcome: Option<CleanupApplyOutcome>,
}

impl WriteHuman for CleanupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if let Some(apply) = &self.apply_outcome {
            if apply.feature_removed {
                writeln!(w, "Cleaned up feature `{}` in {}", apply.feature, self.root)?;
            } else {
                writeln!(
                    w,
                    "Cleaned up feature `{}` in {} — partially; record kept for retry",
                    apply.feature, self.root
                )?;
                for removal in &apply.worktrees {
                    if !removal.removed {
                        writeln!(w, "  {}: worktree not removed", removal.repo)?;
                    }
                }
                for deletion in &apply.branches {
                    if !deletion.deleted {
                        writeln!(w, "  {}: local branch not deleted", deletion.repo)?;
                    }
                }
            }
            Ok(())
        } else {
            writeln!(w, "Cleanup preview for feature `{}`:", self.preview.feature)?;
            for repo in &self.preview.repos {
                let state = if repo.is_delivered {
                    "delivered"
                } else {
                    "not delivered"
                };
                writeln!(w, "  `{}`: {state}", repo.repo)?;
            }
            if self.preview.blockers.is_empty() {
                writeln!(w, "Eligible for cleanup.")?;
            } else {
                writeln!(w, "Blocked by:")?;
                for blocker in &self.preview.blockers {
                    writeln!(w, "  {blocker:?}")?;
                }
            }
            if !self.preview.paths_to_remove.is_empty() {
                writeln!(w, "Paths to remove:")?;
                for path in &self.preview.paths_to_remove {
                    writeln!(w, "  {path}")?;
                }
            }
            writeln!(w, "Fingerprint: {}", self.preview.fingerprint)
        }
    }
}

pub fn cleanup(ctx: &Ctx, input: CleanupInput) -> Outcome<CleanupOutcome> {
    if !input.preview {
        let Some(record_path) = &input.record else {
            return Err(Failure::blocked(
                "feature.cleanup_record_required",
                "cleanup apply requires `--record <path>`",
            ));
        };

        return apply_cleanup(
            ctx,
            &input.feature,
            record_path,
            input.session_id.as_deref(),
        );
    }

    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(input.feature)?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    let previewed = preview_cleanup(
        &git,
        &layout,
        &manifest,
        &feature,
        input.session_id.as_deref(),
    )?;

    Ok(Report::new(CleanupOutcome {
        root: layout.root().to_path_buf(),
        preview: previewed.preview,
        apply_outcome: None,
    }))
}

fn apply_cleanup(
    ctx: &Ctx,
    feature_arg: &str,
    record_path: &Utf8Path,
    own_session: Option<&str>,
) -> Result<Report<CleanupOutcome>, Failure> {
    let layout = discover_hall(ctx)?;
    let record = load_and_validate_record(record_path, &layout)?;

    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(feature_arg.to_owned())?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    let PreviewedCleanup {
        preview,
        forge_consulted,
        worktrees,
        descendants,
    } = preview_cleanup(&git, &layout, &manifest, &feature, own_session)?;

    validate_record_against_preview(&record, &preview, forge_consulted)?;
    preflight_deletable(&layout, &feature, &descendants)?;

    let (descendant_outcomes, mut warnings, all_descendants_removed) =
        teardown_descendants(&layout, &git, &descendants)?;
    if !all_descendants_removed {
        let apply_outcome = CleanupApplyOutcome {
            feature: feature.name.clone(),
            branch: feature.branch,
            fingerprint: preview.fingerprint.clone(),
            worktrees: Vec::new(),
            branches: Vec::new(),
            feature_removed: false,
            descendants: descendant_outcomes,
        };
        return Ok(Report::with_warnings(
            CleanupOutcome {
                root: layout.root().to_path_buf(),
                preview,
                apply_outcome: Some(apply_outcome),
            },
            warnings,
        ));
    }

    let (worktree_removals, root_warnings, all_worktrees_removed) =
        teardown_worktrees(&layout, &git, &feature, &worktrees)?;
    let (branch_deletions, branch_warnings, all_branches_deleted) =
        teardown_branches(&layout, &git, &feature, &worktree_removals);
    warnings.extend(root_warnings);
    warnings.extend(branch_warnings);

    let complete_success = all_worktrees_removed && all_branches_deleted;
    if !complete_success {
        let apply_outcome = CleanupApplyOutcome {
            feature: feature.name.clone(),
            branch: feature.branch,
            fingerprint: preview.fingerprint.clone(),
            worktrees: worktree_removals,
            branches: branch_deletions,
            feature_removed: false,
            descendants: descendant_outcomes,
        };
        return Ok(Report::with_warnings(
            CleanupOutcome {
                root: layout.root().to_path_buf(),
                preview,
                apply_outcome: Some(apply_outcome),
            },
            warnings,
        ));
    }

    // Complete success: remove feature directory, then update durable record outcome
    remove_node_dir(&layout, &feature.name)?;

    let apply_outcome = CleanupApplyOutcome {
        feature: feature.name.clone(),
        branch: feature.branch,
        fingerprint: preview.fingerprint.clone(),
        worktrees: worktree_removals,
        branches: branch_deletions,
        feature_removed: true,
        descendants: descendant_outcomes,
    };

    write_record_outcome(&layout, record, record_path, &apply_outcome)?;

    Ok(Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview,
            apply_outcome: Some(apply_outcome),
        },
        warnings,
    ))
}

fn teardown_descendants(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    descendants: &[(Feature, WorktreeLookups)],
) -> Result<(Vec<CleanupDescendantOutcome>, Vec<Warning>, bool), Failure> {
    let mut warnings = Vec::new();
    let mut descendant_outcomes = Vec::new();
    for (node, node_worktrees) in descendants.iter().rev() {
        let (worktrees, node_warnings, worktrees_removed) =
            teardown_worktrees(layout, git, node, node_worktrees)?;
        let (branches, branch_warnings, branches_deleted) =
            teardown_branches(layout, git, node, &worktrees);
        warnings.extend(node_warnings);
        warnings.extend(branch_warnings);
        let mut removed = false;
        if worktrees_removed && branches_deleted {
            match remove_node_dir(layout, &node.name) {
                Ok(()) => {
                    removed = true;
                }
                Err(failure) => {
                    warnings.push(Warning::new(
                        "feature.cleanup_dir_failed",
                        node.name.as_str(),
                        failure.what.clone(),
                    ));
                }
            }
        }
        descendant_outcomes.push(CleanupDescendantOutcome {
            feature: node.name.clone(),
            branch: node.branch.clone(),
            worktrees,
            branches,
            feature_removed: removed,
        });
        if !removed {
            return Ok((descendant_outcomes, warnings, false));
        }
    }
    Ok((descendant_outcomes, warnings, true))
}

fn write_record_outcome(
    layout: &crate::store::layout::Layout,
    mut record: CleanupRecord,
    record_path: &Utf8Path,
    apply_outcome: &CleanupApplyOutcome,
) -> Result<(), Failure> {
    let abs_record_path = if record_path.is_absolute() {
        record_path.to_path_buf()
    } else {
        layout.root().join(record_path)
    };
    record.outcome = Some(apply_outcome.clone());
    let record_json = json::to_canonical_string(&record)?;
    fs::write_atomic(&abs_record_path, record_json.as_bytes()).map_err(|source| {
        Failure::failed(
            "feature.cleanup_record_write_failed",
            format!("could not write cleanup outcome to record `{record_path}`: {source}"),
        )
    })
}

fn remove_node_dir(
    layout: &crate::store::layout::Layout,
    name: &FeatureName,
) -> Result<(), Failure> {
    fs::remove_path(&layout.feature_dir(name)).map_err(|source| {
        Failure::failed(
            "feature.cleanup_dir_failed",
            format!("could not remove feature `{name}`: {source}"),
        )
    })?;
    let db_path = layout.ivar_dir().join("memory.db");
    if db_path.is_file()
        && let Ok(db) = crate::store::graph::db::GraphDb::open(db_path.as_std_path())
    {
        let _ = db.drop_feature_layers(name.as_str());
    }
    Ok(())
}

fn load_and_validate_record(
    record_path: &Utf8Path,
    layout: &crate::store::layout::Layout,
) -> Result<CleanupRecord, Failure> {
    let docs_updates = layout.docs_updates_dir();

    let abs_record_path = if record_path.is_absolute() {
        record_path.to_path_buf()
    } else {
        layout.root().join(record_path)
    };

    let canonical_updates = fs_err::canonicalize(&docs_updates).map_err(|_| {
        Failure::blocked(
            "feature.cleanup_record_outside_docs_updates",
            format!("cleanup record `{record_path}` must resolve inside `{docs_updates}`"),
        )
    })?;

    let canonical_record = fs_err::canonicalize(&abs_record_path).map_err(|_| {
        Failure::blocked(
            "feature.cleanup_record_not_found",
            format!("cleanup record `{record_path}` does not exist"),
        )
    })?;

    if !canonical_record.starts_with(&canonical_updates) {
        return Err(Failure::blocked(
            "feature.cleanup_record_outside_docs_updates",
            format!("cleanup record `{record_path}` must resolve inside `{docs_updates}`"),
        ));
    }

    let content = crate::infra::fs::read_text(&abs_record_path)
        .map_err(|err| {
            Failure::blocked(
                "feature.cleanup_record_not_found",
                format!("failed to read cleanup record `{record_path}`: {err}"),
            )
        })?
        .ok_or_else(|| {
            Failure::blocked(
                "feature.cleanup_record_not_found",
                format!("cleanup record `{record_path}` does not exist"),
            )
        })?;

    let record: CleanupRecord = serde_json::from_str(&content).map_err(|err| {
        Failure::blocked(
            "feature.cleanup_record_malformed",
            format!("failed to parse cleanup record `{record_path}`: {err}"),
        )
    })?;

    record.validate().map_err(|err| {
        Failure::blocked(
            "feature.cleanup_record_invalid",
            format!("cleanup record at `{record_path}` is invalid: {err}"),
        )
    })?;

    Ok(record)
}

fn validate_record_against_preview(
    record: &CleanupRecord,
    preview: &CleanupPreview,
    forge_consulted: bool,
) -> Result<(), Failure> {
    if record.feature != preview.feature || record.branch != preview.branch {
        return Err(Failure::blocked(
            "feature.cleanup_record_feature_mismatch",
            format!(
                "cleanup record feature `{}` (branch `{}`) does not match feature `{}` (branch `{}`)",
                record.feature, record.branch, preview.feature, preview.branch
            ),
        ));
    }

    if record.fingerprint != preview.fingerprint {
        let mut failure = Failure::blocked(
            "feature.cleanup_fingerprint_mismatch",
            format!(
                "the state of feature `{}` has drifted since the cleanup record was written",
                preview.feature
            ),
        )
        .expected(format!("record fingerprint `{}`", record.fingerprint))
        .actual(format!(
            "current preview fingerprint `{}`",
            preview.fingerprint
        ))
        .fix(FixAction::safe(
            "feature.cleanup_re_preview",
            format!(
                "Rerun `/ivar-feature-cleanup {}` to update the docs and record with the new fingerprint.",
                preview.feature
            ),
        ));
        if forge_consulted {
            failure = failure.fix(FixAction::safe(
                "feature.cleanup_forge_moved",
                "A repo's delivery rests on a pull-request lookup, and the forge now answers differently than it did for the record. Re-run the preview to see the forge's current answer.",
            ));
        }
        return Err(failure);
    }

    if !record.approvals.delivery.approved {
        return Err(Failure::blocked(
            "feature.cleanup_delivery_not_approved",
            format!(
                "delivery approval is false in cleanup record for feature `{}`",
                preview.feature
            ),
        ));
    }

    if !record.approvals.teardown.approved {
        return Err(Failure::blocked(
            "feature.cleanup_teardown_not_approved",
            format!(
                "teardown approval is false in cleanup record for feature `{}`",
                preview.feature
            ),
        ));
    }

    if preview.has_blockers() {
        let command = format!("ivar feature cleanup {} --preview", preview.feature);
        return Err(Failure::blocked(
            "feature.cleanup_blocked",
            format!(
                "feature `{}` cannot be cleaned up due to blockers",
                preview.feature
            ),
        )
        .fix(
            FixAction::safe(
                "feature.cleanup_resolve_blockers",
                format!("List the blockers with `{command}`, resolve them, then clean up again."),
            )
            .command(command),
        ));
    }

    Ok(())
}

fn preflight_deletable(
    layout: &crate::store::layout::Layout,
    feature: &Feature,
    descendants: &[(Feature, WorktreeLookups)],
) -> Result<(), Failure> {
    let mut all_blockers = Vec::new();
    all_blockers.extend(delete::collect_blockers(&layout.feature_dir(&feature.name)));
    for (node, _) in descendants {
        all_blockers.extend(delete::collect_blockers(&layout.feature_dir(&node.name)));
    }
    if !all_blockers.is_empty() {
        let details = serde_json::to_value(&all_blockers).unwrap_or(serde_json::Value::Null);
        return Err(Failure::blocked(
            "feature.delete_blocked",
            format!(
                "cannot clean up feature `{}`: {} path(s) under its directory are not removable",
                feature.name,
                all_blockers.len()
            ),
        )
        .expected("every directory under the feature directory to be writable and searchable")
        .actual(format!(
            "{} path(s) could not be removed — see details for paths, modes, and owners",
            all_blockers.len()
        ))
        .fix(FixAction::safe(
            "feature.fix_permissions",
            "Fix the permissions named above, then run apply again.",
        ))
        .details(details));
    }

    Ok(())
}

fn teardown_worktrees(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &Feature,
    worktrees: &WorktreeLookups,
) -> Result<(Vec<WorktreeRemoval>, Vec<Warning>, bool), Failure> {
    let mut warnings = Vec::new();
    let mut worktree_removals = Vec::new();
    let mut all_worktrees_removed = true;

    for repo in feature.promotions.keys() {
        let worktree = match worktrees.get(repo) {
            Some(Ok(entry)) => entry.as_ref(),
            Some(Err(failure)) => return Err(failure.clone()),
            None => None,
        };
        let Some(worktree) = worktree else {
            worktree_removals.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            });
            continue;
        };
        match git::remove_worktree_entry(git, &layout.repo_bare(repo), worktree) {
            Ok(()) => {
                fs::prune_empty_parents(&worktree.path, &layout.repo_dir(repo));
                worktree_removals.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: true,
                    detail: None,
                });
            }
            Err(error) => {
                all_worktrees_removed = false;
                let detail = error.to_string();
                warnings.push(Warning::new(
                    "feature.cleanup_worktree_failed",
                    repo.as_str(),
                    detail.clone(),
                ));
                worktree_removals.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: false,
                    detail: Some(detail),
                });
            }
        }
    }

    Ok((worktree_removals, warnings, all_worktrees_removed))
}

fn teardown_branches(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &Feature,
    worktree_removals: &[WorktreeRemoval],
) -> (Vec<BranchDeletion>, Vec<Warning>, bool) {
    let mut warnings = Vec::new();
    let mut branch_deletions = Vec::new();
    let mut all_branches_deleted = true;

    for (repo, removal) in feature.promotions.keys().zip(worktree_removals) {
        if !removal.removed {
            all_branches_deleted = false;
            branch_deletions.push(BranchDeletion {
                repo: repo.clone(),
                deleted: false,
                detail: Some("worktree removal failed".to_owned()),
            });
            continue;
        }

        let bare = layout.repo_bare(repo);
        let branch_exists = git.revision_commit(&bare, feature.branch.as_str()).is_ok();
        if !branch_exists {
            // Already absent — idempotent success
            branch_deletions.push(BranchDeletion {
                repo: repo.clone(),
                deleted: true,
                detail: None,
            });
            continue;
        }

        match git.delete_branch(&bare, feature.branch.as_str()) {
            Ok(()) => {
                branch_deletions.push(BranchDeletion {
                    repo: repo.clone(),
                    deleted: true,
                    detail: None,
                });
            }
            Err(error) => {
                all_branches_deleted = false;
                let detail = error.to_string();
                warnings.push(Warning::new(
                    "feature.cleanup_branch_failed",
                    repo.as_str(),
                    detail.clone(),
                ));
                branch_deletions.push(BranchDeletion {
                    repo: repo.clone(),
                    deleted: false,
                    detail: Some(detail),
                });
            }
        }
    }

    (branch_deletions, warnings, all_branches_deleted)
}

/// A cleanup preview, and whether producing it rested on a forge answer.
struct PreviewedCleanup {
    preview: CleanupPreview,
    /// Some repo's verdict came from a live pull-request lookup, which can
    /// answer differently between the preview run and the apply run.
    forge_consulted: bool,
    worktrees: WorktreeLookups,
    descendants: Vec<(Feature, WorktreeLookups)>,
}

/// Each promoted repo's worktree for the feature branch, looked up once per
/// command; the error is kept as text so preview and apply can each report it.
type WorktreeLookups = BTreeMap<RepoName, Result<Option<WorktreeEntry>, Failure>>;

fn lookup_worktrees(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    feature: &Feature,
) -> WorktreeLookups {
    feature
        .promotions
        .keys()
        .map(|repo| {
            let lookup =
                git::lookup_worktree(git, &layout.repo_bare(repo), feature.branch.as_str())
                    .map_err(|error| match error {
                        git::Error::Fs(_) => Failure::from(error),
                        _ => Failure::failed(
                            "feature.cleanup_worktree_lookup_failed",
                            error.to_string(),
                        ),
                    });
            (repo.clone(), lookup)
        })
        .collect()
}

fn removal_paths(
    layout: &crate::store::layout::Layout,
    feature: &Feature,
    repos: &[CleanupRepo],
    worktrees: &WorktreeLookups,
) -> Vec<Utf8PathBuf> {
    let mut paths = Vec::new();
    for repo in repos {
        match worktrees.get(&repo.repo) {
            Some(Ok(Some(entry))) => paths.push(entry.path.clone()),
            _ if !layout.repo_bare(&repo.repo).is_dir() => {
                paths.push(layout.repo_worktree(&repo.repo, &feature.branch));
            }
            _ => {}
        }
    }
    paths.push(layout.feature_dir(&feature.name));
    paths
}

fn live_sessions_of(
    layout: &crate::store::layout::Layout,
    name: &FeatureName,
    own_session: Option<&str>,
) -> (Vec<SessionId>, Option<String>) {
    match session_lookup::list_feature(layout, name) {
        Ok(sessions) => (
            sessions
                .into_iter()
                .filter(|session| own_session != Some(session.id.as_str()))
                .map(|session| session.id)
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

fn preview_cleanup(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
    own_session: Option<&str>,
) -> Result<PreviewedCleanup, Failure> {
    let (live_sessions, session_inspection_error) =
        live_sessions_of(layout, &feature.name, own_session);
    let worktrees = lookup_worktrees(git, layout, feature);
    let repo_facts: Vec<_> = feature
        .promotions
        .iter()
        .map(|(repo, promotion)| {
            let worktree = worktrees.get(repo).unwrap_or(&Ok(None));
            collect_repo_facts(
                git, layout, manifest, feature, repo, promotion, worktree, true,
            )
        })
        .collect();
    let facts = CleanupFacts {
        repos: repo_facts,
        live_sessions,
        session_inspection_error,
    };
    let verdict = classify_cleanup(&facts);
    let repos = facts
        .repos
        .iter()
        .filter_map(cleanup_repo)
        .collect::<Vec<_>>();
    let forge_consulted = facts.repos.iter().any(|repo| repo.forge_delivery.is_some());
    let paths_to_remove = removal_paths(layout, feature, &repos, &worktrees);

    let all = relations::read_all(layout)?;
    let map = relations::build_feature_map(&all);
    let mut descendants = Vec::new();
    let mut descendant_worktrees = Vec::new();
    for (depth, node) in relations::descendants_from_values(&map, &feature.name) {
        let state = relations::feature_state_with_map(git, layout, manifest, &map, node)?;
        let (live_sessions, session_inspection_error) =
            live_sessions_of(layout, &node.name, own_session);
        let node_worktrees = lookup_worktrees(git, layout, node);
        let node_facts = CleanupDescendantFacts {
            state,
            repos: node
                .promotions
                .iter()
                .map(|(repo, promotion)| {
                    let worktree = node_worktrees.get(repo).unwrap_or(&Ok(None));
                    collect_repo_facts(
                        git, layout, manifest, node, repo, promotion, worktree, false,
                    )
                })
                .collect(),
            live_sessions,
            session_inspection_error,
        };
        let node_verdict = classify_descendant_cleanup(&node_facts);
        let node_repos = node_facts
            .repos
            .iter()
            .filter_map(cleanup_repo)
            .collect::<Vec<_>>();
        let node_paths = removal_paths(layout, node, &node_repos, &node_worktrees);
        descendants.push(CleanupDescendant {
            feature: node.name.clone(),
            branch: node.branch.clone(),
            depth,
            parent: node.parent.clone().unwrap_or_else(|| feature.name.clone()),
            state,
            repos: node_repos,
            blockers: node_verdict.blockers,
            paths_to_remove: node_paths,
        });
        descendant_worktrees.push((node.clone(), node_worktrees));
    }

    let fingerprint = fingerprint_for(
        &feature.name,
        &feature.branch,
        &repos,
        &verdict.blockers,
        &paths_to_remove,
        &descendants,
    )?;

    Ok(PreviewedCleanup {
        preview: CleanupPreview {
            feature: feature.name.clone(),
            branch: feature.branch.clone(),
            repos,
            blockers: verdict.blockers,
            paths_to_remove,
            fingerprint,
            descendants,
        },
        forge_consulted,
        worktrees,
        descendants: descendant_worktrees,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_repo_facts(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
    repo: &RepoName,
    promotion: &crate::domain::feature::Promotion,
    worktree_lookup: &Result<Option<WorktreeEntry>, Failure>,
    consult_forge: bool,
) -> CleanupRepoFacts {
    let Some(manifest_repo) = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
    else {
        return absent_manifest_facts(repo);
    };
    let effective_base = base::resolve(feature, promotion, manifest_repo.default_branch());
    let bare = layout.repo_bare(repo);
    let clone_exists = matches!(git.target_state(&bare), Ok(TargetState::Repository));
    let mut inspection_error = None;
    let (feature_head, base_head, local_branch_exists, unmerged_commits, forge_delivery) =
        if clone_exists {
            let feature_head = revision(git, &bare, feature.branch.as_str(), &mut inspection_error);
            let base_head = revision(git, &bare, effective_base.as_str(), &mut inspection_error);
            let local_branch_exists = feature_head.is_some();
            let unmerged_commits = match base::unmerged_commits(
                git,
                &bare,
                effective_base.as_str(),
                feature.branch.as_str(),
            ) {
                Ok(commits) => Some(commits),
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    None
                }
            };
            let forge_delivery = if consult_forge {
                match (unmerged_commits, feature_head.as_deref()) {
                    (Some(1..), Some(head)) => {
                        Some(ask_forge(&bare, feature.branch.as_str(), head))
                    }
                    _ => None,
                }
            } else {
                None
            };
            (
                feature_head,
                base_head,
                local_branch_exists,
                unmerged_commits,
                forge_delivery,
            )
        } else {
            (None, None, false, None, None)
        };
    let worktree = match worktree_lookup {
        Ok(entry) => entry.as_ref().filter(|entry| !entry.prunable),
        Err(failure) => {
            inspection_error.get_or_insert_with(|| failure.what.clone());
            None
        }
    };
    let worktree_exists = worktree.is_some();
    let dirty_worktree = if let Some(worktree) = worktree {
        match git.worktree_dirty(&worktree.path) {
            Ok(dirty) => Some(dirty),
            Err(error) => {
                inspection_error.get_or_insert_with(|| error.to_string());
                None
            }
        }
    } else {
        None
    };

    CleanupRepoFacts {
        repo: repo.clone(),
        effective_base: Some(effective_base),
        feature_head,
        base_head,
        local_branch_exists,
        worktree_exists,
        clone_exists,
        dirty_worktree,
        unmerged_commits,
        in_manifest: true,
        inspection_error,
        forge_delivery,
    }
}

/// Ask the forge about `branch`, and read its answer as a delivery verdict.
fn ask_forge(bare: &Utf8Path, branch: &str, feature_head: &str) -> ForgeDelivery {
    match pull_requests::list_pull_requests(bare, branch, "all") {
        Ok(prs) => read_forge_answer(&prs, feature_head),
        Err(failure) => ForgeDelivery::Unavailable {
            reason: failure.what,
        },
    }
}

/// The verdict the forge's pull requests support for `feature_head`. A merged
/// pull request for this exact head wins over every earlier record for the
/// branch.
fn read_forge_answer(prs: &[PullRequest], feature_head: &str) -> ForgeDelivery {
    if prs.iter().any(|pr| pr.merged_head(feature_head)) {
        return ForgeDelivery::Merged;
    }
    let Some(pr) = prs.iter().find(|pr| pr.is_merged()).or(prs.first()) else {
        return ForgeDelivery::NoPullRequest;
    };
    if pr.is_merged() {
        ForgeDelivery::MergedOtherHead { number: pr.number }
    } else {
        ForgeDelivery::NotMerged {
            number: pr.number,
            state: pr.state.clone(),
        }
    }
}

fn absent_manifest_facts(repo: &RepoName) -> CleanupRepoFacts {
    CleanupRepoFacts {
        repo: repo.clone(),
        effective_base: None,
        feature_head: None,
        base_head: None,
        local_branch_exists: false,
        worktree_exists: false,
        clone_exists: false,
        dirty_worktree: None,
        unmerged_commits: None,
        in_manifest: false,
        inspection_error: None,
        forge_delivery: None,
    }
}

fn revision(
    git: &impl Git,
    bare: &Utf8Path,
    branch: &str,
    error: &mut Option<String>,
) -> Option<String> {
    match git.revision_commit(bare, branch) {
        Ok(commit) => Some(commit),
        Err(cause) => {
            error.get_or_insert_with(|| cause.to_string());
            None
        }
    }
}

fn cleanup_repo(facts: &CleanupRepoFacts) -> Option<CleanupRepo> {
    let effective_base = facts.effective_base.as_ref()?;
    Some(CleanupRepo {
        repo: facts.repo.clone(),
        effective_base: effective_base.clone(),
        feature_head: facts.feature_head.clone(),
        base_head: facts.base_head.clone(),
        local_branch_exists: facts.local_branch_exists,
        worktree_exists: facts.worktree_exists,
        is_delivered: facts.in_manifest
            && facts.clone_exists
            && facts.to_delivery_facts().unmerged_commits == Some(0),
    })
}

/// Forge answers are live network text (timeouts, auth prompts) that can differ
/// between preview and apply; the blocker itself still enters the fingerprint.
fn without_forge_detail(blocker: &CleanupBlocker) -> CleanupBlocker {
    let mut blocker = blocker.clone();
    if let CleanupBlocker::UnmergedCommits { forge, .. } = &mut blocker {
        *forge = None;
    }
    blocker
}

fn fingerprint_for(
    feature: &FeatureName,
    branch: &crate::domain::name::BranchName,
    repos: &[CleanupRepo],
    blockers: &[CleanupBlocker],
    paths_to_remove: &[Utf8PathBuf],
    descendants: &[CleanupDescendant],
) -> Result<String, Failure> {
    let preview = CleanupPreview {
        feature: feature.clone(),
        branch: branch.clone(),
        repos: repos.to_vec(),
        blockers: blockers.iter().map(without_forge_detail).collect(),
        paths_to_remove: paths_to_remove.to_vec(),
        fingerprint: String::new(),
        descendants: descendants
            .iter()
            .map(|node| CleanupDescendant {
                blockers: node.blockers.iter().map(without_forge_detail).collect(),
                ..node.clone()
            })
            .collect(),
    };
    Ok(hash::text(&json::to_canonical_string(&preview)?))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/cleanup.rs"]
mod tests;
