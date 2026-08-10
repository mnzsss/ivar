#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with one synced repo (`api`, default branch `main`).
fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
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
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    (guard, root)
}

fn setup_input(repo: &str) -> SetupInput {
    SetupInput {
        repo: repo.to_owned(),
        force: false,
    }
}

/// Write a setup script that leaves a marker file in the worktree it runs
/// in.
fn write_setup_script(root: &Utf8PathBuf) {
    fs::ensure_dir(&root.join(".ivar/setups")).unwrap();
    fs::write_text(
        &root.join(".ivar/setups/api.sh"),
        "#!/usr/bin/env bash\ntouch setup-ran\n",
    )
    .unwrap();
}

#[test]
fn setup_runs_the_script_the_first_time() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    write_setup_script(&root);

    let report = setup(&ctx, setup_input("api")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.change, Some(Change::Created));
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/main/setup-ran")).unwrap(),
        ""
    );
}

/// The receipt is respected: a second run with the same script content
/// does not re-run the script, even though its effect was undone.
#[test]
fn setup_respects_the_receipt_and_does_not_re_run() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    write_setup_script(&root);

    setup(&ctx, setup_input("api")).unwrap();
    // Undo the script's effect behind ivar's back.
    fs::remove_file(&root.join(".ivar/repos/api/main/setup-ran")).unwrap();

    let report = setup(&ctx, setup_input("api")).unwrap();

    assert_eq!(report.value.change, Some(Change::Unchanged));
    assert!(
        !root.join(".ivar/repos/api/main/setup-ran").exists(),
        "the receipt must skip a script whose content has not changed"
    );
}

/// `--force-setup` ignores the receipt: the same unchanged script runs.
#[test]
fn setup_force_ignores_the_receipt() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    write_setup_script(&root);

    setup(&ctx, setup_input("api")).unwrap();
    fs::remove_file(&root.join(".ivar/repos/api/main/setup-ran")).unwrap();

    let report = setup(
        &ctx,
        SetupInput {
            repo: "api".to_owned(),
            force: true,
        },
    )
    .unwrap();

    assert_eq!(report.value.change, Some(Change::Updated));
    assert!(
        root.join(".ivar/repos/api/main/setup-ran").exists(),
        "force must run the script even though its content is unchanged"
    );
}

/// A script whose content changed is run again — content, not mtime.
#[test]
fn setup_reruns_a_script_whose_content_changed() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    write_setup_script(&root);
    setup(&ctx, setup_input("api")).unwrap();

    fs::write_text(
        &root.join(".ivar/setups/api.sh"),
        "#!/usr/bin/env bash\ntouch setup-ran-v2\n",
    )
    .unwrap();

    let report = setup(&ctx, setup_input("api")).unwrap();

    assert_eq!(report.value.change, Some(Change::Updated));
    assert!(
        root.join(".ivar/repos/api/main/setup-ran-v2").exists(),
        "a changed script must run again"
    );
}

#[test]
fn setup_of_a_repo_without_a_script_is_an_explained_no_op() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let report = setup(&ctx, setup_input("api")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.change, None);
    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    assert!(
        String::from_utf8(out).unwrap().contains("nothing to run"),
        "the no-op must be explained, not silent"
    );
}

#[test]
fn setup_is_refused_for_a_repo_not_in_the_manifest() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let failure = setup(&ctx, setup_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.setup_repo_not_found");
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn setup_is_refused_when_the_worktree_is_missing() {
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
    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
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
    // Declared but never synced — no worktree exists. The script exists,
    // so this is a worktree problem, not a no-script no-op.
    fs::ensure_dir(&root.join(".ivar/setups")).unwrap();
    fs::write_text(
        &root.join(".ivar/setups/api.sh"),
        "#!/usr/bin/env bash\ntouch x\n",
    )
    .unwrap();

    let failure = setup(&ctx, setup_input("api")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.setup_worktree_missing");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
    drop(guard);
}

#[test]
fn the_human_surface_names_what_happened() {
    let outcome = SetupOutcome {
        root: Utf8PathBuf::from("/hall"),
        repo: RepoName::new("api").unwrap(),
        script: Utf8PathBuf::from("/hall/.ivar/setups/api.sh"),
        change: Some(Change::Unchanged),
        detail: Some("already run for this version of the script".to_owned()),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Setup script for `api` not run — already run for this version of the script.\n"
    );
}
