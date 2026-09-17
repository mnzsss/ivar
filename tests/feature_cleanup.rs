//! Integration tests for `ivar feature cleanup --preview` against a forge.
//!
//! The forge answer is what decides whether a branch ahead of its base is
//! nonetheless delivered, and reading it means running `gh`. These tests drive
//! the compiled binary with the fake `gh` first on `PATH`, which is the only
//! way that path is exercised without the real GitHub CLI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

#[path = "delivery/support.rs"]
mod support;

use camino::Utf8PathBuf;
use common::{FakeGh, hall_root};
use support::{ivar_on_github, setup_deliver_hall};

const PR_URL: &str = "https://github.com/acme/pull/1";

fn preview_with_pr_state(state: &str) -> serde_json::Value {
    let (guard, root) = hall_root();
    setup_deliver_hall(&root);
    let fake = FakeGh::install(&root);
    let bare: Utf8PathBuf = root.join(".ivar/repos/api/.bare");
    fake.set_existing_pr(&bare, "checkout", PR_URL, "main", state);

    let output = ivar_on_github(&fake, &[])
        .current_dir(&root)
        .args(["feature", "cleanup", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(
        fake.log().contains("pr list --head checkout --state all"),
        "cleanup must ask the forge about the branch, log was: {}",
        fake.log()
    );
    drop(guard);
    serde_json::from_slice(&output).expect("valid json")
}

fn unmerged_blocker(preview: &serde_json::Value) -> Option<&serde_json::Value> {
    preview["blockers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|blocker| blocker["kind"] == "unmerged_commits")
}

#[test]
fn a_merged_pull_request_for_the_local_head_makes_the_repo_delivered() {
    let value = preview_with_pr_state("MERGED");
    let preview = &value["preview"];

    assert_eq!(preview["repos"][0]["repo"], "api");
    assert_eq!(
        preview["repos"][0]["is_delivered"], true,
        "a merged PR for this exact head delivers the repo: {preview}"
    );
    assert!(
        unmerged_blocker(preview).is_none(),
        "the forge answer must remove the unmerged-commits blocker: {preview}"
    );
}

#[test]
fn an_open_pull_request_keeps_the_blocker_and_names_the_forge_reason() {
    let value = preview_with_pr_state("OPEN");
    let preview = &value["preview"];

    assert_eq!(preview["repos"][0]["is_delivered"], false);
    let blocker = unmerged_blocker(preview).expect("unmerged-commits blocker");
    assert_eq!(blocker["repo"], "api");
    assert_eq!(blocker["effective_base"], "main");
    assert_eq!(blocker["commits"], 1);
    let forge = blocker["forge"].as_str().expect("a forge reason");
    assert!(
        forge.contains('1') && forge.to_lowercase().contains("open"),
        "the forge reason must name the pull request and its state, got: {forge}"
    );
}
