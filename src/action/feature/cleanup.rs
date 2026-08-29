//! `ivar feature cleanup` — build the side-effect-free cleanup preview.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::action::feature::delete;
use crate::action::session::lookup as session_lookup;
use crate::domain::feature::{
    BranchDeletion, CleanupApplyOutcome, CleanupBlocker, CleanupFacts, CleanupPreview,
    CleanupRecord, CleanupRepo, CleanupRepoFacts, Feature, WorktreeRemoval, classify_cleanup,
};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::{fs, hash, json};

use super::super::{discover_hall, read_manifest};
use super::{base, relations};

#[derive(Debug, Clone)]
pub struct CleanupInput {
    pub feature: String,
    pub preview: bool,
    pub record: Option<Utf8PathBuf>,
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

        return apply_cleanup(ctx, &input.feature, record_path);
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
    let preview = preview_for(&git, &layout, &manifest, &feature)?;

    Ok(Report::new(CleanupOutcome {
        root: layout.root().to_path_buf(),
        preview,
        apply_outcome: None,
    }))
}

fn apply_cleanup(
    ctx: &Ctx,
    feature_arg: &str,
    record_path: &camino::Utf8Path,
) -> Result<Report<CleanupOutcome>, Failure> {
    let layout = discover_hall(ctx)?;
    let docs_updates = layout.docs_updates_dir();

    // 1. Validate record path: must resolve inside <hall>/docs/updates/
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

    // 2. Read and parse record JSON
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

    // 3. Validate intrinsic record field rules
    record.validate().map_err(|err| {
        Failure::blocked(
            "feature.cleanup_record_invalid",
            format!("cleanup record at `{record_path}` is invalid: {err}"),
        )
    })?;

    // 4. Compute preview for feature
    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(feature_arg.to_owned())?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    let preview = preview_for(&git, &layout, &manifest, &feature)?;

    // 5. Check record feature == preview feature and record branch == preview branch
    if record.feature != preview.feature || record.branch != preview.branch {
        return Err(Failure::blocked(
            "feature.cleanup_record_feature_mismatch",
            format!(
                "cleanup record feature `{}` (branch `{}`) does not match feature `{}` (branch `{}`)",
                record.feature, record.branch, preview.feature, preview.branch
            ),
        ));
    }

    // 6. Check fingerprint comparison
    if record.fingerprint != preview.fingerprint {
        return Err(Failure::blocked(
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
        )));
    }

    // 7. Check approvals: delivery & teardown
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

    // 8. Check preview blockers
    if !preview.blockers.is_empty() {
        return Err(Failure::blocked(
            "feature.cleanup_blocked",
            format!(
                "feature `{}` cannot be cleaned up due to blockers",
                preview.feature
            ),
        ));
    }

    // 9. Preflight: descendants check (defensive)
    let descendants = relations::descendants(&layout, &feature.name)?;
    if !descendants.is_empty() {
        let names = descendants
            .iter()
            .map(|descendant| descendant.name.to_string())
            .collect::<Vec<_>>();
        return Err(Failure::blocked(
            "feature.has_descendants",
            format!(
                "cannot delete feature `{}`: it has {} descendant(s)",
                feature.name,
                descendants.len()
            ),
        )
        .expected("every descendant to be deleted first")
        .actual(format!("descendants: {}", names.join(", ")))
        .fix(FixAction::safe(
            "feature.delete_leaves_first",
            "Delete the descendants first, leaves first.",
        )));
    }

    // 10. Preflight: permissions check on feature directory
    let blockers = delete::collect_blockers(&layout.feature_dir(&feature.name));
    if !blockers.is_empty() {
        let details = serde_json::to_value(&blockers).unwrap_or(serde_json::Value::Null);
        return Err(Failure::blocked(
            "feature.delete_blocked",
            format!(
                "cannot clean up feature `{}`: {} path(s) under its directory are not removable",
                feature.name,
                blockers.len()
            ),
        )
        .expected("every path under the feature directory to be writable and searchable")
        .actual(format!(
            "{} path(s) could not be removed — see details for paths, modes, and owners",
            blockers.len()
        ))
        .fix(FixAction::safe(
            "feature.fix_permissions",
            "Fix the permissions named above, then run apply again.",
        ))
        .details(details));
    }

    // 11. Teardown: worktree removal loop
    let mut warnings = Vec::new();
    let mut worktree_removals = Vec::new();
    let mut all_worktrees_removed = true;

    for repo in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo, &feature.branch);
        if !fs::is_dir(&worktree)? {
            worktree_removals.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            });
            continue;
        }
        match git.remove_worktree(&layout.repo_bare(repo), &worktree) {
            Ok(()) => {
                fs::prune_empty_parents(&worktree, &layout.repo_dir(repo));
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

    // 12. Teardown: local branch deletion loop
    let mut branch_deletions = Vec::new();
    let mut all_branches_deleted = true;

    for (repo, removal) in feature.promotions.keys().zip(&worktree_removals) {
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

    let complete_success = all_worktrees_removed && all_branches_deleted;

    if !complete_success {
        let apply_outcome = CleanupApplyOutcome {
            feature: feature.name.clone(),
            branch: feature.branch.clone(),
            fingerprint: preview.fingerprint.clone(),
            worktrees: worktree_removals,
            branches: branch_deletions,
            feature_removed: false,
            plans_removed: false,
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

    // Complete success: remove plans and feature directory, then update durable record outcome
    fs::remove_path(&layout.plan_dir(&feature.name)).map_err(|source| {
        Failure::failed(
            "feature.cleanup_plans_failed",
            format!(
                "could not remove plans for feature `{}`: {source}",
                feature.name
            ),
        )
    })?;
    fs::remove_path(&layout.feature_dir(&feature.name)).map_err(|source| {
        Failure::failed(
            "feature.cleanup_dir_failed",
            format!("could not remove feature `{}`: {source}", feature.name),
        )
    })?;

    let apply_outcome = CleanupApplyOutcome {
        feature: feature.name.clone(),
        branch: feature.branch.clone(),
        fingerprint: preview.fingerprint.clone(),
        worktrees: worktree_removals,
        branches: branch_deletions,
        feature_removed: true,
        plans_removed: true,
    };

    let mut record = record;
    record.outcome = Some(apply_outcome.clone());
    let record_json = json::to_canonical_string(&record)?;
    fs::write_atomic(&abs_record_path, record_json.as_bytes()).map_err(|source| {
        Failure::failed(
            "feature.cleanup_record_write_failed",
            format!("could not write cleanup outcome to record `{record_path}`: {source}"),
        )
    })?;

    Ok(Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview,
            apply_outcome: Some(apply_outcome),
        },
        warnings,
    ))
}

fn preview_for(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
) -> Result<CleanupPreview, Failure> {
    let (live_sessions, session_inspection_error) =
        match session_lookup::list_feature(layout, &feature.name) {
            Ok(sessions) => (
                sessions.into_iter().map(|session| session.id).collect(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
    let descendants = relations::descendants(layout, &feature.name)?
        .into_iter()
        .map(|descendant| descendant.name)
        .collect();

    let repo_facts: Vec<_> = feature
        .promotions
        .iter()
        .map(|(repo, promotion)| {
            collect_repo_facts(git, layout, manifest, feature, repo, promotion)
        })
        .collect();
    let facts = CleanupFacts {
        repos: repo_facts,
        live_sessions,
        descendants,
        session_inspection_error,
    };
    let verdict = classify_cleanup(&facts);
    let repos = facts
        .repos
        .iter()
        .filter_map(cleanup_repo)
        .collect::<Vec<_>>();
    let mut paths_to_remove = Vec::new();
    for repo in &repos {
        paths_to_remove.push(layout.repo_worktree(&repo.repo, &feature.branch));
    }
    paths_to_remove.push(layout.plan_dir(&feature.name));
    paths_to_remove.push(layout.feature_dir(&feature.name));

    let fingerprint = fingerprint_for(
        &feature.name,
        &feature.branch,
        &repos,
        &verdict.blockers,
        &paths_to_remove,
    )?;

    Ok(CleanupPreview {
        feature: feature.name.clone(),
        branch: feature.branch.clone(),
        repos,
        blockers: verdict.blockers,
        paths_to_remove,
        fingerprint,
    })
}

fn collect_repo_facts(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
    repo: &RepoName,
    promotion: &crate::domain::feature::Promotion,
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
    let worktree = layout.repo_worktree(repo, &feature.branch);
    let clone_exists = matches!(git.target_state(&bare), Ok(TargetState::Repository));
    let worktree_exists = matches!(git.target_state(&worktree), Ok(TargetState::Repository));
    let mut inspection_error = None;
    let (feature_head, base_head, local_branch_exists, unmerged_commits) = if clone_exists {
        let feature_head = revision(git, &bare, feature.branch.as_str(), &mut inspection_error);
        let base_head = revision(git, &bare, effective_base.as_str(), &mut inspection_error);
        let local_branch_exists = feature_head.is_some();
        let unmerged_commits =
            match git.commits_ahead(&bare, effective_base.as_str(), feature.branch.as_str()) {
                Ok(commits) => Some(commits),
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    None
                }
            };
        (
            feature_head,
            base_head,
            local_branch_exists,
            unmerged_commits,
        )
    } else {
        (None, None, false, None)
    };
    let dirty_worktree = if worktree_exists {
        match git.worktree_dirty(&worktree) {
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
    }
}

fn revision(
    git: &impl Git,
    bare: &camino::Utf8Path,
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
        is_delivered: facts.in_manifest && facts.clone_exists && facts.unmerged_commits == Some(0),
    })
}

fn fingerprint_for(
    feature: &FeatureName,
    branch: &crate::domain::name::BranchName,
    repos: &[CleanupRepo],
    blockers: &[CleanupBlocker],
    paths_to_remove: &[Utf8PathBuf],
) -> Result<String, Failure> {
    let preview = CleanupPreview {
        feature: feature.clone(),
        branch: branch.clone(),
        repos: repos.to_vec(),
        blockers: blockers.to_vec(),
        paths_to_remove: paths_to_remove.to_vec(),
        fingerprint: String::new(),
    };
    Ok(hash::text(&json::to_canonical_string(&preview)?))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/cleanup.rs"]
mod tests;
