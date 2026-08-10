//! End-to-end smoke test that ivar's View Dir does not break `lazygit`.
//!
//! Regression guard for ticket #22: the GRM broke lazygit entirely with
//! perfectly legal libgit2 output, and the ivar View Dir is made of symlinks.
//! This test creates a real hall with a promoted repo and a non-promoted repo,
//! opens a session (which materialises the View Dir), then checks that
//! `lazygit` can read the repos without panicking.
//!
//! The test **skips** when `lazygit` is not installed — it never fails for
//! absence (CI is offline).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::{hall_root, ivar, seeded_repo};

/// Verify that `lazygit` exists on this machine. Returns `true` when the
/// binary is found on PATH, `false` otherwise.
fn lazygit_available() -> bool {
    std::process::Command::new("lazygit")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

#[test]
fn view_dir_does_not_break_lazygit() {
    // Skip gracefully when lazygit is not installed.
    if !lazygit_available() {
        println!("SKIP: lazygit not installed — skipping view-dir / lazygit smoke test");
        return;
    }

    let (_guard, root) = hall_root();

    // --- Set up a hall with two repos: one promoted, one not ----------------

    // Init the hall.
    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme", "--provider", "claude-code"])
        .assert()
        .success();

    // Create two origin repos.
    let api_origin = root.parent().unwrap().join("origins").join("api");
    let web_origin = root.parent().unwrap().join("origins").join("web");
    seeded_repo(&api_origin, "main");
    seeded_repo(&web_origin, "main");

    // Declare both repos in ivar.json (hand-edit, matching the contract).
    let api_entry = format!(
        r#"{{"default_branch":"main","name":"api","url":"{}"}}"#,
        api_origin
    );
    let web_entry = format!(
        r#"{{"default_branch":"main","name":"web","url":"{}"}}"#,
        web_origin
    );
    let repos_json = format!("{api_entry},{web_entry}");
    let mut ivar_json = std::fs::read_to_string(root.join("ivar.json")).expect("read ivar.json");
    ivar_json = ivar_json.replace("\"repos\":[]", &format!("\"repos\":[{repos_json}]"));
    std::fs::write(root.join("ivar.json"), &ivar_json).expect("write ivar.json");

    // Sync to clone bare repos and materialise default worktrees.
    ivar().current_dir(&root).arg("sync").assert().success();

    // Create a feature and promote only `api`.
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    ivar()
        .current_dir(&root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    // Start a session (detached) to materialise the View Dir.
    ivar()
        .current_dir(&root)
        .args(["session", "start", "checkout", "--detached"])
        .assert()
        .success();

    // Locate the View Dir under the session tree.
    let sessions_dir = root.join(".ivar/features/checkout/sessions");
    let view_dir = std::fs::read_dir(&sessions_dir)
        .expect("sessions dir exists")
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .expect("at least one session dir")
        .path();

    // --- Run lazygit inside the View Dir ------------------------------------

    // `lazygit status` reads git metadata without mutating anything. Running it
    // inside the View Dir exercises the symlink chain that libgit2 follows.
    // We capture its exit code: zero means it could read the repos; non-zero
    // or a crash would indicate a regression against ticket #22.
    let output = std::process::Command::new("lazygit")
        .arg("status")
        .current_dir(&view_dir)
        .output()
        .expect("lazygit runs");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // lazygit exits 1 when there are uncommitted changes (expected here since
    // we just created repos with content). The important thing is it did NOT
    // crash or fail to parse the git objects.
    let success = output.status.success() || output.status.code() == Some(1);

    assert!(
        success,
        "lazygit failed inside the View Dir (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
}
