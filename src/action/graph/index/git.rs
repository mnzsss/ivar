//! Git repository inspection and incremental diff detection.

use std::collections::HashSet;

use super::types::{IndexError, is_ignored_path, is_supported_file};

/// Retrieves the current HEAD commit OID.
pub fn get_head_oid(git_repo: &git2::Repository) -> Option<git2::Oid> {
    git_repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .or_else(|| {
            git_repo
                .find_reference("HEAD")
                .ok()
                .and_then(|r| r.peel_to_commit().ok())
                .map(|c| c.id())
        })
}

/// Checks if the working tree has changes to supported code files since the last indexed commit.
pub fn is_up_to_date(
    git_repo: &git2::Repository,
    head_oid: git2::Oid,
    last_indexed: &str,
) -> Result<bool, IndexError> {
    if head_oid.to_string() != *last_indexed {
        return Ok(false);
    }

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.include_untracked(true);
    diff_opts.recurse_untracked_dirs(true);

    let head_commit = git_repo.find_commit(head_oid)?;
    let head_tree = head_commit.tree()?;

    let last_oid = git2::Oid::from_str(last_indexed).ok();
    let diff_target_tree = if let Some(last_oid) = last_oid
        && let Ok(last_commit) = git_repo.find_commit(last_oid)
    {
        last_commit.tree().ok()
    } else {
        None
    };

    let mut diff = git_repo
        .diff_tree_to_workdir_with_index(diff_target_tree.as_ref(), Some(&mut diff_opts))?;

    if let Some(target_tree) = diff_target_tree.as_ref() {
        let tree_to_tree_diff = git_repo.diff_tree_to_tree(
            Some(target_tree),
            Some(&head_tree),
            Some(&mut diff_opts),
        )?;
        diff.merge(&tree_to_tree_diff)?;
    }

    let mut has_relevant_changes = false;
    let res = diff.foreach(
        &mut |delta, _| {
            let path = delta.new_file().path().or_else(|| delta.old_file().path());
            if let Some(p) = path
                && is_supported_file(p)
                && !is_ignored_path(p)
                && !git_repo.is_path_ignored(p).unwrap_or(false)
            {
                has_relevant_changes = true;
                return false;
            }
            true
        },
        None,
        None,
        None,
    );

    if let Err(e) = res
        && e.code() != git2::ErrorCode::User
    {
        return Err(IndexError::from(e));
    }

    Ok(!has_relevant_changes)
}

/// Computes lists of files to index and files to delete relative to the last indexed commit.
pub fn compute_git_diff(
    git_repo: &git2::Repository,
    last_head_str: &str,
) -> Result<(Vec<String>, Vec<String>), IndexError> {
    let last_oid = git2::Oid::from_str(last_head_str).ok();
    let last_tree = if let Some(oid) = last_oid {
        git_repo.find_commit(oid).ok().and_then(|c| c.tree().ok())
    } else {
        None
    };

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.include_untracked(true);
    diff_opts.recurse_untracked_dirs(true);

    let diff =
        git_repo.diff_tree_to_workdir_with_index(last_tree.as_ref(), Some(&mut diff_opts))?;

    let mut files_to_index = Vec::new();
    let mut files_to_delete = Vec::new();
    let mut seen_indexed = HashSet::new();
    let mut seen_deleted = HashSet::new();

    diff.foreach(
        &mut |delta, _| {
            match delta.status() {
                git2::Delta::Deleted => {
                    if let Some(path) = delta.old_file().path()
                        && is_supported_file(path)
                        && !is_ignored_path(path)
                    {
                        let path_str = path.to_string_lossy().to_string();
                        if seen_deleted.insert(path_str.clone()) {
                            files_to_delete.push(path_str);
                        }
                    }
                }
                git2::Delta::Added
                | git2::Delta::Modified
                | git2::Delta::Untracked
                | git2::Delta::Typechange
                | git2::Delta::Renamed
                | git2::Delta::Copied => {
                    if let Some(old_path) = delta.old_file().path()
                        && delta.status() == git2::Delta::Renamed
                        && is_supported_file(old_path)
                        && !is_ignored_path(old_path)
                    {
                        let old_path_str = old_path.to_string_lossy().to_string();
                        if seen_deleted.insert(old_path_str.clone()) {
                            files_to_delete.push(old_path_str);
                        }
                    }
                    if let Some(new_path) = delta.new_file().path()
                        && is_supported_file(new_path)
                        && !is_ignored_path(new_path)
                        && !git_repo.is_path_ignored(new_path).unwrap_or(false)
                    {
                        let path_str = new_path.to_string_lossy().to_string();
                        if seen_indexed.insert(path_str.clone()) {
                            files_to_index.push(path_str);
                        }
                    }
                }
                _ => {}
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok((files_to_index, files_to_delete))
}
