//! Isolated Git memory auto-commit implementation.

use camino::Utf8PathBuf;

use crate::domain::memory::writeset::MemoryWriteSet;
use crate::error::Failure;
use crate::infra::proc;
use crate::store::layout::Layout;

struct TempIndexGuard(std::path::PathBuf);

impl Drop for TempIndexGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Outcome of an automated memory commit attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCommitOutcome {
    /// Commit SHA produced, or None if no commit was created.
    pub commit_sha: Option<String>,
    /// Paths included in the commit.
    pub committed_paths: Vec<Utf8PathBuf>,
    /// Whether concurrency caused a CAS failure, requiring a retry.
    pub retry_pending: bool,
}

/// Perform an isolated automated Git commit of memory files modified during a session.
///
/// Uses an isolated `GIT_INDEX_FILE` so the primary Git index (`.git/index`) and staging
/// area remain untouched.
pub fn auto_commit_memory(
    layout: &Layout,
    writeset: &MemoryWriteSet,
) -> Result<MemoryCommitOutcome, Failure> {
    if writeset.is_empty() {
        return Ok(MemoryCommitOutcome {
            commit_sha: None,
            committed_paths: Vec::new(),
            retry_pending: false,
        });
    }

    let existing_writeset = writeset.filter_existing(layout.root());
    if existing_writeset.is_empty() {
        return Ok(MemoryCommitOutcome {
            commit_sha: None,
            committed_paths: Vec::new(),
            retry_pending: false,
        });
    }

    let root = layout.root();

    // Check if git is initialized in the hall root.
    if !crate::infra::fs::exists(&root.join(".git")).unwrap_or(false) {
        return Ok(MemoryCommitOutcome {
            commit_sha: None,
            committed_paths: Vec::new(),
            retry_pending: false,
        });
    }

    let temp_dir_path = root
        .as_std_path()
        .join(format!(".ivar-git-index-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir_path).map_err(|err| {
        Failure::failed(
            "git.memory_commit.temp_index_failed",
            format!("failed to create temporary git index directory: {err}"),
        )
    })?;
    let _guard = TempIndexGuard(temp_dir_path.clone());
    let temp_index_file = temp_dir_path.join("index");
    let temp_index_path = temp_index_file.to_str().ok_or_else(|| {
        Failure::failed(
            "git.memory_commit.invalid_path",
            "temporary git index path is not valid utf-8",
        )
    })?;

    let git_cmd = |args: &[&str]| {
        proc::Command::new("git")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .env("GIT_INDEX_FILE", temp_index_path)
            .cwd(root)
            .args(args.iter().copied())
    };

    // a. Get current HEAD commit sha via `git rev-parse HEAD` (if git repo has HEAD).
    let head_output = proc::capture(&git_cmd(&["rev-parse", "HEAD"])).map_err(|err| {
        Failure::failed(
            "git.memory_commit.exec_failed",
            format!("git rev-parse HEAD failed: {err}"),
        )
    })?;
    let head_sha = if head_output.success() {
        let sha = head_output.stdout.trim().to_owned();
        if sha.is_empty() { None } else { Some(sha) }
    } else {
        None
    };

    // b. Populate temp index: if HEAD exists, run `git read-tree HEAD`.
    if let Some(head) = &head_sha {
        let read_tree = proc::capture(&git_cmd(&["read-tree", head])).map_err(|err| {
            Failure::failed(
                "git.memory_commit.exec_failed",
                format!("git read-tree failed: {err}"),
            )
        })?;
        if !read_tree.success() {
            return Err(Failure::failed(
                "git.memory_commit.read_tree_failed",
                format!("git read-tree HEAD failed: {}", read_tree.stderr),
            ));
        }
    }

    // c. Add only the writeset.modified_paths using `git add -- <paths>`.
    let mut add_args = vec!["add", "--"];
    let path_strings: Vec<String> = existing_writeset
        .modified_paths
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    for p in &path_strings {
        add_args.push(p.as_str());
    }

    let add_out = proc::capture(&git_cmd(&add_args)).map_err(|err| {
        Failure::failed(
            "git.memory_commit.exec_failed",
            format!("git add failed: {err}"),
        )
    })?;
    if !add_out.success() {
        return Err(Failure::failed(
            "git.memory_commit.add_failed",
            format!("git add failed: {}", add_out.stderr),
        ));
    }

    // d. Write tree: `git write-tree` to get `<tree_sha>`.
    let write_tree_out = proc::capture(&git_cmd(&["write-tree"])).map_err(|err| {
        Failure::failed(
            "git.memory_commit.exec_failed",
            format!("git write-tree failed: {err}"),
        )
    })?;
    if !write_tree_out.success() {
        return Err(Failure::failed(
            "git.memory_commit.write_tree_failed",
            format!("git write-tree failed: {}", write_tree_out.stderr),
        ));
    }
    let tree_sha = write_tree_out.stdout.trim().to_owned();

    // e. If `<tree_sha>` equals HEAD's tree sha, nothing changed; return outcome with `commit_sha: None`.
    if let Some(head) = &head_sha {
        let head_tree_out = proc::capture(&git_cmd(&["rev-parse", &format!("{head}^{{tree}}")]))
            .map_err(|err| {
                Failure::failed(
                    "git.memory_commit.exec_failed",
                    format!("git rev-parse tree failed: {err}"),
                )
            })?;
        if head_tree_out.success() && head_tree_out.stdout.trim() == tree_sha {
            return Ok(MemoryCommitOutcome {
                commit_sha: None,
                committed_paths: Vec::new(),
                retry_pending: false,
            });
        }
    }

    // f. Create commit: `git commit-tree <tree_sha> [-p <head_sha>] -m "chore(memory): auto-commit session <session_id>"`
    let commit_msg = format!("chore(memory): auto-commit session {}", writeset.session);
    let mut commit_args = vec!["commit-tree", tree_sha.as_str()];
    if let Some(head) = &head_sha {
        commit_args.push("-p");
        commit_args.push(head.as_str());
    }
    commit_args.push("-m");
    commit_args.push(commit_msg.as_str());

    let commit_out = proc::capture(&git_cmd(&commit_args)).map_err(|err| {
        Failure::failed(
            "git.memory_commit.exec_failed",
            format!("git commit-tree failed: {err}"),
        )
    })?;
    if !commit_out.success() {
        return Err(Failure::failed(
            "git.memory_commit.commit_tree_failed",
            format!("git commit-tree failed: {}", commit_out.stderr),
        ));
    }
    let new_commit_sha = commit_out.stdout.trim().to_owned();

    // g. CAS ref update: `git update-ref HEAD <new_sha> <old_head_sha>`.
    let mut update_args = vec!["update-ref", "HEAD", new_commit_sha.as_str()];
    if let Some(head) = &head_sha {
        update_args.push(head.as_str());
    } else {
        // If initial commit with no prior HEAD, old ref is empty or 0000000000000000000000000000000000000000 / ""
        update_args.push("");
    }

    let update_out = proc::capture(&git_cmd(&update_args)).map_err(|err| {
        Failure::failed(
            "git.memory_commit.exec_failed",
            format!("git update-ref failed: {err}"),
        )
    })?;

    if update_out.success() {
        Ok(MemoryCommitOutcome {
            commit_sha: Some(new_commit_sha),
            committed_paths: existing_writeset.modified_paths,
            retry_pending: false,
        })
    } else {
        // Concurrency / CAS failure
        Ok(MemoryCommitOutcome {
            commit_sha: None,
            committed_paths: Vec::new(),
            retry_pending: true,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/git/memory_commit.rs"]
mod tests;
