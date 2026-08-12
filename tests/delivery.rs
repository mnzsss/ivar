//! Integration tests for `ivar feature deliver` — PR creation and sibling linking.
//!
//! These tests drive the compiled binary to verify:
//! - Preview distinguishes `PushOnly` from create-PR (`new_pr`)
//! - The fingerprint still gates apply
//! - A repo's failure becomes a `Warning`, not a batch abort
//!
//! The `gh` interaction is exercised indirectly: when `gh` is not on PATH,
//! PR creation fails with a spawn error which surfaces as a `Warning` (not an
//! abort), proving best-effort semantics. When `gh` IS available, the created
//! PR URL appears in the preview's `pr_url` field.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use camino::{Utf8Path, Utf8PathBuf};
use common::{declare_repos, hall_root, ivar, seeded_repo};
use predicates::prelude::*;

/// Run git in `cwd` with a fixed identity.
fn git(cwd: &Utf8Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");

    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Set up a hall with one promoted repo, synced, feature created, repo promoted,
/// and one commit on the feature branch. Returns (guard, root).
fn setup_deliver_hall(root: &Utf8PathBuf) {
    ivar().current_dir(root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(root, &[("api", &origin, "main")]);
    ivar().current_dir(root).arg("sync").assert().success();

    // Create a feature and promote the repo.
    ivar()
        .current_dir(root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    ivar()
        .current_dir(root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    // Add a commit on the feature branch so there is something to deliver.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
}

/// Walk `feature` through the SPDD gates up to and including `plan`, using only
/// CLI verbs. This is the path a human takes; `deliver` refuses without it.
fn approve_through_plan(root: &Utf8PathBuf, feature: &str) {
    ivar()
        .current_dir(root)
        .args(["plan", "create", feature])
        .assert()
        .success();

    for gate in ["requirements", "analysis", "plan"] {
        ivar()
            .current_dir(root)
            .args(["plan", "approve", feature, gate])
            .assert()
            .success();
    }
}

/// The fingerprint `--preview` printed for `feature`.
fn preview_fingerprint(root: &Utf8PathBuf, feature: &str) -> String {
    let output = ivar()
        .current_dir(root)
        .args(["feature", "deliver", feature, "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    value["preview"]["fingerprint"]
        .as_str()
        .expect("fingerprint is a string")
        .to_owned()
}

// -- preview ------------------------------------------------------------------

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    assert_eq!(preview["feature"], "checkout");
    let repos = preview["repos"].as_array().expect("repos is an array");
    assert_eq!(repos.len(), 1);
    let repo = &repos[0];
    assert_eq!(repo["repo"], "api");
    assert_eq!(repo["local_branch"], "checkout");
    assert!(repo["remote"].as_str().unwrap().contains("origins/api"));
    assert_eq!(repo["push_refspec"], "checkout:refs/heads/checkout");
    // The remote is a local path, not GitHub — push only, no PR surface.
    assert_eq!(repo["action"], "push_only");
    assert_eq!(repo["base_branch"], "main");
    assert!(repo["dependencies"].is_array());
    // There should be a blocker about unpushed commits.
    let blockers = repo["blockers"].as_array().expect("blockers is an array");
    assert!(
        blockers.iter().any(|b| {
            let s = b.as_str().unwrap_or("");
            s.contains("commit") || s.contains("push")
        }),
        "expected an unpushed-commits blocker, got: {blockers:?}"
    );
    // No pr_url in preview (nothing created yet).
    assert!(repo["pr_url"].is_null());
    // Fingerprint is present and non-empty.
    assert!(
        preview["fingerprint"].as_str().unwrap().len() == 64,
        "fingerprint must be a sha-256 hex digest"
    );
}

#[test]
fn preview_for_a_feature_with_no_promoted_repos_is_empty() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);
    ivar().current_dir(&root).arg("sync").assert().success();

    ivar()
        .current_dir(&root)
        .args(["feature", "create", "empty"])
        .assert()
        .success();

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "empty", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let _value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["preview"]["repos"].as_array().unwrap().len(), 0);
    assert!(value["preview"]["fingerprint"].as_str().unwrap().len() == 64);
}

// -- apply: gating ------------------------------------------------------------

#[test]
fn apply_requires_a_preview_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("preview fingerprint"));
}

#[test]
fn apply_is_rejected_when_the_fingerprint_does_not_match() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Get a valid preview first.
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let good_fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Drift: add another commit.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("more.md"), "more\n").unwrap();
    git(&worktree, &["add", "more.md"]);
    git(&worktree, &["commit", "-m", "more"]);

    // Apply with the OLD fingerprint → blocked.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &good_fp])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}

// -- apply: the plan gate -----------------------------------------------------

#[test]
fn apply_is_refused_while_the_plan_gate_is_not_approved() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("plan"))
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn apply_is_refused_even_with_a_matching_fingerprint_while_the_gate_is_pending() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    // A fingerprint straight off the preview — the drift gate is satisfied, and
    // the plan gate still refuses. The two are independent.
    let fp = preview_fingerprint(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &fp])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn approving_only_the_upstream_gates_is_not_enough() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    ivar()
        .current_dir(&root)
        .args(["plan", "create", "checkout"])
        .assert()
        .success();
    for gate in ["requirements", "analysis"] {
        ivar()
            .current_dir(&root)
            .args(["plan", "approve", "checkout", gate])
            .assert()
            .success();
    }

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn invalidating_the_plan_gate_closes_delivery_again() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args(["plan", "invalidate", "checkout", "plan"])
        .assert()
        .success();

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn the_preview_reports_the_plan_gate_without_refusing() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["preview"]["plan_gate"], "pending");

    approve_through_plan(&root, "checkout");

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["preview"]["plan_gate"], "approved");
}

#[test]
fn approving_the_plan_after_a_preview_drifts_the_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let stale = preview_fingerprint(&root, "checkout");
    approve_through_plan(&root, "checkout");

    // The gate state is part of what the human approved, so crossing it is
    // drift like any other — the preview has to be taken again.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &stale])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}

// -- apply: pushing -----------------------------------------------------------

#[test]
fn deliver_pushes_the_feature_branch_to_the_remote() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Preview to get the fingerprint.
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Apply.
    let output = ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["pushes"].as_array().unwrap().len(), 1);
    assert_eq!(value["pushes"][0]["repo"], "api");
    assert_eq!(value["pushes"][0]["ok"], true);
}

#[test]
fn a_failed_push_becomes_a_warning_not_an_abort() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Break the remote before delivering.
    let manifest_path = root.join("ivar.json");
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    let modified = content.replace(
        "\"url\":\"",
        &format!("\"url\":\"{}", root.join("no-such-origin")),
    );
    std::fs::write(&manifest_path, modified).unwrap();

    // Preview to get the fingerprint (with the broken URL baked in).
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Apply — should succeed overall but with a warning.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &fp])
        .assert()
        .code(1) // exit 1 = warnings present
        .stderr(
            predicate::str::contains("does not appear to be a git repository")
                .or(predicate::str::contains("could not read from remote")),
        );
}

// -- human surface ------------------------------------------------------------

#[test]
fn human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let human = String::from_utf8(output).expect("utf8 output");
    assert!(human.contains("Delivery preview for `checkout`"));
    assert!(human.contains("branch:  checkout"));
    assert!(human.contains("refspec: checkout:refs/heads/checkout"));
    assert!(human.contains("base:    main"));
    // The remote is a local path — push only, no PR.
    assert!(human.contains("action:  push only"));
    assert!(human.contains("fingerprint:"));
}

// -- end to end ---------------------------------------------------------------

/// Zero to delivered, through CLI verbs only.
///
/// The point of this test is what it does *not* do: no file under `.ivar/` is
/// written by hand at any step. Every state the run passes through — the hall,
/// the repo, the feature, the promotion, the four planning artifacts, the three
/// crossed gates, the preview fingerprint — is reachable by running `ivar`.
/// A gate that could only be crossed by editing JSON would fail here.
#[test]
fn the_whole_path_from_an_empty_directory_to_a_pushed_branch_runs_on_cli_verbs_only() {
    let (_guard, root) = hall_root();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");

    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["repo", "add", "api", origin.as_str()])
        .assert()
        .success();
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

    // Something to deliver.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    approve_through_plan(&root, "checkout");

    let fingerprint = preview_fingerprint(&root, "checkout");
    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fingerprint,
        ])
        .assert()
        .success();

    // The branch actually landed on the origin.
    let output = std::process::Command::new("git")
        .args(["-C", origin.as_str(), "rev-parse", "--verify"])
        .arg("refs/heads/checkout")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "the feature branch never reached the origin: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// -- apply: pull requests -----------------------------------------------------
//
// A GitHub remote is what makes `deliver` reach for `gh` at all, and no test
// may reach the network. Both halves are faked at the process boundary: a fake
// `gh` on `PATH` that answers exactly what the real CLI answers, and git's
// `insteadOf` (set on the child's environment) pointing the `https://github.com`
// URL back at the local origin the rest of the file already builds.

/// A fake `gh` on `PATH`, implementing the contract delivery relies on:
/// `pr list --head … --json url`, `pr create`, and `pr comment`. Open PRs are
/// keyed by (working directory, head branch), so two repos sharing a feature
/// branch name do not collide.
///
/// It refuses `--json` on `pr create` because the real `gh` does — that flag
/// exists on `pr list`/`pr view` and nowhere else — and it refuses to create a
/// second PR for a branch that already has one, also like the real `gh`.
struct FakeGh {
    dir: Utf8PathBuf,
    state: Utf8PathBuf,
    log: Utf8PathBuf,
}

const FAKE_GH: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$GH_FAKE_LOG"

sub="$1 $2"
shift 2

head=""
base=""
json=0
while [ $# -gt 0 ]; do
  case "$1" in
    --head) head="$2"; shift 2 ;;
    --base) base="$2"; shift 2 ;;
    --json) json=1; shift 2 ;;
    *) shift ;;
  esac
done

key="$(pwd) $head"
url=$(grep -F "$key " "$GH_FAKE_STATE" | head -n 1 | awk '{print $NF}')

case "$sub" in
  "pr list")
    if [ -n "$url" ]; then printf '[{"url":"%s"}]\n' "$url"; else printf '[]\n'; fi
    ;;
  "pr create")
    if [ "$json" -eq 1 ]; then
      printf 'unknown flag: --json\n' >&2
      exit 1
    fi
    if [ -n "$url" ]; then
      printf 'a pull request for branch "%s" into branch "%s" already exists:\n%s\n' \
        "$head" "$base" "$url" >&2
      exit 1
    fi
    number=$(( $(wc -l < "$GH_FAKE_STATE") + 1 ))
    url="https://github.com/acme/pull/$number"
    printf '%s %s\n' "$key" "$url" >> "$GH_FAKE_STATE"
    printf '%s\n' "$url"
    ;;
  "pr comment")
    ;;
  *)
    printf 'unknown command: %s\n' "$sub" >&2
    exit 1
    ;;
esac
exit 0
"#;

impl FakeGh {
    fn install(root: &Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let gh = dir.join("gh");
        std::fs::write(&gh, FAKE_GH).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let state = dir.join("prs");
        let log = dir.join("log");
        std::fs::write(&state, "").unwrap();
        std::fs::write(&log, "").unwrap();
        Self { dir, state, log }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }
}

/// Point every declared repo at a `https://github.com/…` URL — the only kind
/// that gets a PR — while keeping the real origin reachable through git's
/// `insteadOf`. Returns the config pairs the child process needs.
fn as_github_remotes(root: &Utf8Path) -> Vec<(String, String)> {
    let path = root.join("ivar.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut rewrites = Vec::new();
    for repo in value["repos"].as_array_mut().unwrap() {
        let name = repo["name"].as_str().unwrap().to_owned();
        let origin = repo["url"].as_str().unwrap().to_owned();
        let url = format!("https://github.com/acme/{name}.git");
        rewrites.push((format!("url.{origin}.insteadOf"), url.clone()));
        repo["url"] = serde_json::Value::String(url);
    }
    std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
    rewrites
}

/// `ivar`, with the fake `gh` first on `PATH` and the remote rewrites in place.
fn ivar_on_github(fake: &FakeGh, rewrites: &[(String, String)]) -> assert_cmd::Command {
    let mut command = ivar();
    let path = std::env::var("PATH").unwrap_or_default();
    command
        .env("PATH", format!("{}:{path}", fake.dir))
        .env("GH_FAKE_STATE", fake.state.as_str())
        .env("GH_FAKE_LOG", fake.log.as_str())
        .env("GIT_CONFIG_COUNT", rewrites.len().to_string());
    for (index, (key, value)) in rewrites.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command
}

/// Preview `feature` through the fake `gh`, returning the whole JSON document
/// so a caller can read both the predicted action and the fingerprint.
fn preview_on_github(
    root: &Utf8PathBuf,
    fake: &FakeGh,
    rewrites: &[(String, String)],
    feature: &str,
) -> serde_json::Value {
    let output = ivar_on_github(fake, rewrites)
        .current_dir(root)
        .args(["feature", "deliver", feature, "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

/// Deliver `feature` through the fake `gh`, taking the fingerprint from a fresh
/// preview. Returns the apply document.
fn deliver_on_github(
    root: &Utf8PathBuf,
    fake: &FakeGh,
    rewrites: &[(String, String)],
    feature: &str,
) -> serde_json::Value {
    let fingerprint = preview_on_github(root, fake, rewrites, feature)["preview"]["fingerprint"]
        .as_str()
        .expect("fingerprint is a string")
        .to_owned();

    let output = ivar_on_github(fake, rewrites)
        .current_dir(root)
        .args([
            "feature",
            "deliver",
            feature,
            "--fingerprint",
            &fingerprint,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn delivering_a_github_repo_opens_a_pull_request_and_records_its_url() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(preview["preview"]["repos"][0]["action"], "new_pr");

    let applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "the PR URL `gh` printed has to land on the repo"
    );
}

#[test]
fn delivering_again_updates_the_existing_pull_request_instead_of_failing() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second run: the branch already has a PR, so the action flips to
    // `update_pr` — pushing updates it in place, and nothing is recreated.
    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(preview["preview"]["repos"][0]["action"], "update_pr");

    let applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "an update keeps reporting the PR it updated"
    );
    assert_eq!(
        fake.log().matches("pr create").count(),
        1,
        "a second `gh pr create` for a branch that already has a PR is the bug"
    );
}

/// Two promoted repos, both with something to deliver, so the batch has
/// siblings to link.
fn setup_two_repo_hall(root: &Utf8PathBuf) {
    ivar().current_dir(root).arg("init").assert().success();
    let origins = root.parent().unwrap().join("origins");
    let api = seeded_repo(&origins.join("api"), "main");
    let web = seeded_repo(&origins.join("web"), "main");
    declare_repos(root, &[("api", &api, "main"), ("web", &web, "main")]);
    ivar().current_dir(root).arg("sync").assert().success();
    ivar()
        .current_dir(root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    for repo in ["api", "web"] {
        ivar()
            .current_dir(root)
            .args(["feature", "promote", "checkout", repo])
            .assert()
            .success();
        let worktree = root.join(format!(".ivar/repos/{repo}/checkout"));
        std::fs::write(worktree.join("work.md"), "work\n").unwrap();
        git(&worktree, &["add", "work.md"]);
        git(&worktree, &["commit", "-m", "work"]);
    }
}

#[test]
fn sibling_pull_requests_are_linked_to_each_other() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert!(
        log.contains("pr comment https://github.com/acme/pull/1"),
        "the first PR was never told about its sibling: {log}"
    );
    assert!(
        log.contains("pr comment https://github.com/acme/pull/2"),
        "the second PR was never told about its sibling: {log}"
    );
}
