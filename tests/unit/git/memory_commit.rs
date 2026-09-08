#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::memory::writeset::MemoryWriteSet;
use crate::domain::name::SessionId;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn setup_git_repo(root: &camino::Utf8Path) {
    let output = std::process::Command::new("git")
        .arg("init")
        .current_dir(root)
        .output()
        .expect("git init");
    assert!(output.status.success());

    // Configure user name and email for commit-tree
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Ivar Test"])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "ivar@test.local"])
        .current_dir(root)
        .output();
}

#[test]
fn auto_commit_memory_preserves_staging_area() {
    let (_guard, root) = hall_root();
    setup_git_repo(&root);
    let layout = Layout::at(root.clone());

    // Create initial commit
    let init_file = root.join("README.md");
    crate::infra::fs::write_atomic(&init_file, b"# Readme\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&root)
        .output()
        .unwrap();

    // Stage an unrelated file in the primary git staging area
    let staging_file = root.join("staged_unrelated.txt");
    crate::infra::fs::write_atomic(&staging_file, b"staged content").unwrap();
    std::process::Command::new("git")
        .args(["add", "staged_unrelated.txt"])
        .current_dir(&root)
        .output()
        .unwrap();

    // Modify memory file
    let mem_file = root.join("memory/topics/test.md");
    crate::infra::fs::ensure_dir(mem_file.parent().unwrap()).unwrap();
    crate::infra::fs::write_atomic(&mem_file, b"memory content").unwrap();

    let session = SessionId::new("00000000-0000-0000-0000-000000000001").unwrap();
    let writeset = MemoryWriteSet::new(session, vec![Utf8PathBuf::from("memory/topics/test.md")]);

    let outcome = auto_commit_memory(&layout, &writeset).expect("auto commit memory");
    assert!(outcome.commit_sha.is_some());
    assert_eq!(
        outcome.committed_paths,
        vec![Utf8PathBuf::from("memory/topics/test.md")]
    );
    assert!(!outcome.retry_pending);

    // Verify staged_unrelated.txt is still staged in primary index
    let status_out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status_out.stdout);
    assert!(status.contains("A  staged_unrelated.txt"));
    // memory/topics/test.md was committed to HEAD, so in working tree relative to HEAD it is clean (not staged as A in primary index)
    assert!(!status.contains("A  memory/topics/test.md"));
}

#[test]
fn auto_commit_memory_idempotent_no_changes() {
    let (_guard, root) = hall_root();
    setup_git_repo(&root);
    let layout = Layout::at(root.clone());

    let mem_file = root.join("memory/topics/test.md");
    crate::infra::fs::ensure_dir(mem_file.parent().unwrap()).unwrap();
    crate::infra::fs::write_atomic(&mem_file, b"memory content").unwrap();

    let session = SessionId::new("00000000-0000-0000-0000-000000000001").unwrap();
    let writeset = MemoryWriteSet::new(
        session.clone(),
        vec![Utf8PathBuf::from("memory/topics/test.md")],
    );

    let outcome1 = auto_commit_memory(&layout, &writeset).expect("first auto commit");
    assert!(outcome1.commit_sha.is_some());

    let outcome2 = auto_commit_memory(&layout, &writeset).expect("second auto commit");
    assert_eq!(outcome2.commit_sha, None);
    assert_eq!(outcome2.committed_paths, Vec::<Utf8PathBuf>::new());
    assert!(!outcome2.retry_pending);
}

#[test]
fn auto_commit_memory_cas_concurrency() {
    let (_guard, root) = hall_root();
    setup_git_repo(&root);
    let layout = Layout::at(root.clone());

    // Create initial commit
    let init_file = root.join("README.md");
    crate::infra::fs::write_atomic(&init_file, b"# Readme\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&root)
        .output()
        .unwrap();

    let mem_file = root.join("memory/topics/test.md");
    crate::infra::fs::ensure_dir(mem_file.parent().unwrap()).unwrap();
    crate::infra::fs::write_atomic(&mem_file, b"memory content").unwrap();

    let session = SessionId::new("00000000-0000-0000-0000-000000000001").unwrap();
    let writeset = MemoryWriteSet::new(session, vec![Utf8PathBuf::from("memory/topics/test.md")]);

    // Initial commit succeeds
    let outcome = auto_commit_memory(&layout, &writeset).unwrap();
    assert!(!outcome.retry_pending);
    assert!(outcome.commit_sha.is_some());
}
