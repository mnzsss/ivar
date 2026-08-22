use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8PathBuf;

use crate::domain::feature::{
    PathEvidence, PathState, RepoBaseline, RepoDiff, RunBaseline, RunDiff, classify_change,
};
use crate::git::{Git, System};
use crate::infra::{fs, hash};

pub(super) fn baseline(
    worktrees: &BTreeMap<String, Utf8PathBuf>,
) -> Result<RunBaseline, crate::error::Failure> {
    let git = System;
    let mut repos = BTreeMap::new();
    for (name, worktree) in worktrees {
        let head = git
            .head_commit(worktree)
            .map_err(crate::error::Failure::from)?;
        let dirty = git
            .changed_paths(worktree)
            .map_err(crate::error::Failure::from)?
            .into_iter()
            .map(|path| evidence(worktree.join(&path)).map(|value| (path, value)))
            .collect::<Result<_, _>>()?;
        repos.insert(
            name.clone(),
            RepoBaseline {
                worktree: worktree.clone(),
                head,
                dirty,
            },
        );
    }
    Ok(RunBaseline { repos })
}

pub(super) fn diff(baseline: &RunBaseline) -> Result<RunDiff, crate::error::Failure> {
    let git = System;
    let mut repos = BTreeMap::new();
    for (name, repo) in &baseline.repos {
        let mut paths: BTreeSet<_> = repo.dirty.keys().cloned().collect();
        paths.extend(
            git.changed_paths(&repo.worktree)
                .map_err(crate::error::Failure::from)?,
        );
        paths.extend(
            git.paths_committed_since(&repo.worktree, &repo.head)
                .map_err(crate::error::Failure::from)?,
        );
        let mut changes = BTreeMap::new();
        for path in paths {
            let initial = repo.dirty.get(&path).cloned().unwrap_or_else(|| {
                commit_evidence(&git, repo, &path).unwrap_or_else(|_| PathEvidence::absent())
            });
            let commit = commit_evidence(&git, repo, &path)?;
            let final_state = evidence(repo.worktree.join(&path))?;
            if let Some(kind) = classify_change(&initial, &commit, &final_state) {
                changes.insert(
                    path,
                    crate::domain::feature::PathChange { kind, final_state },
                );
            }
        }
        repos.insert(
            name.clone(),
            RepoDiff {
                head: git
                    .head_commit(&repo.worktree)
                    .map_err(crate::error::Failure::from)?,
                changes,
            },
        );
    }
    Ok(RunDiff { repos })
}

fn commit_evidence(
    git: &System,
    repo: &RepoBaseline,
    path: &camino::Utf8Path,
) -> Result<PathEvidence, crate::error::Failure> {
    Ok(
        match git
            .path_at_commit(&repo.worktree, &repo.head, path)
            .map_err(crate::error::Failure::from)?
        {
            Some(blob) if blob.mode == 0o120_000 => PathEvidence::symlink(blob.sha256),
            Some(blob) => PathEvidence::file(blob.mode, blob.sha256),
            None => PathEvidence::absent(),
        },
    )
}

fn evidence(path: Utf8PathBuf) -> Result<PathEvidence, crate::error::Failure> {
    if !fs::exists(&path)? {
        return Ok(PathEvidence::absent());
    }
    let Some(metadata) = fs::stat(&path)? else {
        return Ok(PathEvidence::absent());
    };
    #[cfg(unix)]
    let mode = std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o177_777;
    #[cfg(not(unix))]
    let mode = 0o100_644;
    if std::fs::symlink_metadata(path.as_std_path())
        .map_err(|source| {
            crate::error::Failure::from(crate::infra::fs::Error::Metadata {
                path: path.clone(),
                source,
            })
        })?
        .file_type()
        .is_symlink()
    {
        return Ok(PathEvidence::symlink(hash::bytes(
            std::fs::read_link(path.as_std_path())
                .map_err(|source| {
                    crate::error::Failure::from(crate::infra::fs::Error::Read {
                        path: path.clone(),
                        source,
                    })
                })?
                .as_os_str()
                .as_encoded_bytes(),
        )));
    }
    Ok(PathEvidence {
        state: PathState::File,
        mode: Some(mode),
        hash: Some(hash::file(&path)?),
    })
}
