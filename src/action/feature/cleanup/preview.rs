use super::*;

/// A cleanup preview, and whether producing it rested on a forge answer.
pub(super) struct PreviewedCleanup {
    pub(super) preview: CleanupPreview,
    /// Some repo's verdict came from a live pull-request lookup, which can
    /// answer differently between the preview run and the apply run.
    pub(super) forge_consulted: bool,
    pub(super) worktrees: WorktreeLookups,
    pub(super) descendants: Vec<(Feature, WorktreeLookups)>,
    pub(super) merged: Vec<(RepoName, PullRequest)>,
}

/// Each promoted repo's worktree for the feature branch, looked up once per
/// command; the error is kept as text so preview and apply can each report it.
pub(super) type WorktreeLookups = BTreeMap<RepoName, Result<Option<WorktreeEntry>, Failure>>;

pub(super) fn lookup_worktrees(
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

pub(super) fn removal_paths(
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

pub(super) fn live_sessions_of(
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

pub(super) fn preview_cleanup(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
    own_session: Option<&str>,
) -> Result<PreviewedCleanup, Failure> {
    let (live_sessions, session_inspection_error) =
        live_sessions_of(layout, &feature.name, own_session);
    let worktrees = lookup_worktrees(git, layout, feature);
    let merged = std::cell::RefCell::new(Vec::new());
    let repo_facts: Vec<_> = feature
        .promotions
        .iter()
        .map(|(repo, promotion)| {
            let worktree = worktrees.get(repo).unwrap_or(&Ok(None));
            collect_repo_facts(
                git, layout, manifest, feature, repo, promotion, worktree, true, &merged,
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
                        git, layout, manifest, node, repo, promotion, worktree, false, &merged,
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
        merged: merged.into_inner(),
    })
}

pub(super) fn absent_manifest_facts(repo: &RepoName) -> CleanupRepoFacts {
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

pub(super) fn revision(
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

pub(super) fn cleanup_repo(facts: &CleanupRepoFacts) -> Option<CleanupRepo> {
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
pub(super) fn without_forge_detail(blocker: &CleanupBlocker) -> CleanupBlocker {
    let mut blocker = blocker.clone();
    if let CleanupBlocker::UnmergedCommits { forge, .. } = &mut blocker {
        *forge = None;
    }
    blocker
}

pub(super) fn fingerprint_for(
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
