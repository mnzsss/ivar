//! Test harness for `ivar graph` integration scenarios.
//!
//! Provides [`GraphHall`] which sets up an isolated temporary hall and a
//! hermetic local clone of the current repository, runs initial setup and indexing,
//! and exposes helper methods for CLI interaction, JSON assertion, stdin piping,
//! and safe worktree mutation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use assert_cmd::Command;
use assert_cmd::assert::Assert;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;
use std::process::Command as StdCommand;
use tempfile::TempDir;

use crate::common::{declare_repos, git, hall_root, ivar, utf8_temp_dir};

/// Locate the current ivar source checkout root from `CARGO_MANIFEST_DIR`.
fn current_ivar_source_dir() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Detect the active branch name of the given git repository, falling back to `main`.
fn detect_current_branch(repo_path: &Utf8Path) -> String {
    let output = StdCommand::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_path)
        .output();

    if let Ok(out) = output
        && out.status.success()
    {
        let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !branch.is_empty() && branch != "HEAD" {
            return branch;
        }
    }
    "main".to_owned()
}

/// Create a non-destructive, isolated local clone of the current ivar checkout
/// in a temporary directory on its current branch.
fn create_isolated_ivar_clone(target_path: &Utf8Path) -> (String, Utf8PathBuf) {
    let source_dir = current_ivar_source_dir();
    let branch = detect_current_branch(&source_dir);

    git(
        target_path.parent().expect("target_path has parent"),
        &[
            "clone",
            "--no-hardlinks",
            source_dir.as_str(),
            target_path.as_str(),
        ],
    );

    git(target_path, &["checkout", "-B", &branch, "HEAD"]);

    (branch, target_path.to_path_buf())
}

/// Hermetic test environment owning an isolated hall and repository clone for `ivar graph`.
pub(crate) struct GraphHall {
    _hall_guard: TempDir,
    _clone_guard: TempDir,
    pub(crate) hall_root: Utf8PathBuf,
    pub(crate) repo_name: String,
    pub(crate) branch: String,
    pub(crate) worktree_path: Utf8PathBuf,
    pub(crate) db_path: Utf8PathBuf,
    pub(crate) initial_index: Value,
}
impl GraphHall {
    /// Create a new `GraphHall` by cloning the current repository into a temporary directory,
    /// initializing the hall, syncing the repository, and performing the initial index.
    pub(crate) fn from_current_repo() -> Self {
        let (hall_guard, hall_dir) = hall_root();
        let (clone_guard, clone_temp_dir) = utf8_temp_dir();
        let origin_path = clone_temp_dir.join("ivar-origin");

        let (current_branch, origin) = create_isolated_ivar_clone(&origin_path);
        let repo_name = "ivar".to_owned();

        // 1. Initialize hall
        let mut init_cmd = ivar();
        init_cmd.current_dir(&hall_dir).arg("init");
        init_cmd.assert().success().code(0);

        // 2. Declare repository
        declare_repos(&hall_dir, &[(&repo_name, &origin, &current_branch)]);

        // 3. Sync repository into the hall
        let mut sync_cmd = ivar();
        sync_cmd.current_dir(&hall_dir).arg("sync");
        sync_cmd.assert().success().code(0);

        let worktree_path = hall_dir
            .join(".ivar")
            .join("repos")
            .join(&repo_name)
            .join(&current_branch);
        assert!(
            worktree_path.exists(),
            "Expected synced worktree at {worktree_path}"
        );

        let db_path = hall_dir.join(".ivar").join("memory.db");

        let mut instance = Self {
            _hall_guard: hall_guard,
            _clone_guard: clone_guard,
            hall_root: hall_dir,
            repo_name,
            branch: current_branch,
            worktree_path,
            db_path,
            initial_index: Value::Null,
        };

        // 4. Initial index
        instance.initial_index = instance.index();
        instance
    }

    /// Pre-configured `Command` targeting the hall root.
    pub(crate) fn ivar(&self) -> Command {
        let mut cmd = ivar();
        cmd.current_dir(&self.hall_root);
        cmd
    }

    /// Trigger `ivar graph index --json` and return the JSON outcome.
    pub(crate) fn index(&self) -> Value {
        self.run_json(&["graph", "index"])
    }

    /// Run `ivar --json <args>`, assert exit code 0, and parse stdout as JSON.
    pub(crate) fn run_json(&self, args: &[&str]) -> Value {
        self.run_json_stdin(args, None)
    }

    /// Run `ivar --json <args>` with stdin input, assert exit code 0, and parse stdout as JSON.
    pub(crate) fn run_json_stdin(&self, args: &[&str], stdin_input: Option<&str>) -> Value {
        let mut cmd = self.ivar();
        cmd.arg("--json").args(args);

        if let Some(input) = stdin_input {
            cmd.write_stdin(input);
        }

        let assert = cmd.assert().success().code(0);
        let stdout = String::from_utf8(assert.get_output().stdout.clone())
            .expect("ivar output should be valid utf-8");

        serde_json::from_str(&stdout).unwrap_or_else(|err| {
            panic!("failed to parse JSON from `ivar --json {args:?}`: {err}\nOutput was: {stdout}")
        })
    }

    /// Run human-formatted `ivar <args>` and return the assertion handle for output inspections.
    pub(crate) fn run_human(&self, args: &[&str]) -> Assert {
        let mut cmd = self.ivar();
        cmd.args(args);
        cmd.assert().success().code(0)
    }

    /// Resolve a normalized relative path inside the synced worktree.
    pub(crate) fn worktree_file(&self, relative_path: &str) -> Utf8PathBuf {
        let rel = Utf8Path::new(relative_path);
        assert!(
            rel.components()
                .all(|component| matches!(component, camino::Utf8Component::Normal(_))),
            "Path must stay inside the worktree, got: {relative_path}"
        );
        self.worktree_path.join(rel)
    }

    /// Read file content from the synced worktree.
    pub(crate) fn read_worktree_file(&self, relative_path: &str) -> String {
        let full_path = self.worktree_file(relative_path);
        std::fs::read_to_string(&full_path)
            .unwrap_or_else(|err| panic!("failed to read worktree file {full_path}: {err}"))
    }

    /// Write content to a file inside the synced worktree.
    pub(crate) fn write_worktree_file(&self, relative_path: &str, content: &str) {
        let full_path = self.worktree_file(relative_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|err| panic!("failed to create dir {parent}: {err}"));
        }
        std::fs::write(&full_path, content)
            .unwrap_or_else(|err| panic!("failed to write worktree file {full_path}: {err}"));
    }

    /// Append content to an existing file inside the synced worktree.
    pub(crate) fn append_worktree_file(&self, relative_path: &str, suffix: &str) {
        let original = self.read_worktree_file(relative_path);
        let mut modified = original;
        modified.push_str(suffix);
        self.write_worktree_file(relative_path, &modified);
    }
}
