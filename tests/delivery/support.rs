//! Delivery-only fixtures and helpers.
//!
//! Shared by every delivery integration-test module. Reuses the existing
//! cross-target `common` support (`hall_root`, `seeded_repo`, `declare_repos`,
//! `FakeGh`, `ivar`, `git`) and adds only the helpers delivery tests need.
//! No duplication of shared infrastructure.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use crate::common::{FakeGh, declare_repos, git, ivar, seeded_repo};
use camino::{Utf8Path, Utf8PathBuf};

/// Walk `feature` through the SPDD gates up to and including `plan`, using only
/// CLI verbs. This is the path a human takes; `deliver` refuses without it.
pub(crate) fn approve_through_plan(root: &Utf8PathBuf, feature: &str) {
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
pub(crate) fn preview_fingerprint(root: &Utf8PathBuf, feature: &str) -> String {
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

/// Point every declared repo at a `https://github.com/…` URL — the only kind
/// that gets a PR — while keeping the real origin reachable through git's
/// `insteadOf`. Returns the config pairs the child process needs.
pub(crate) fn as_github_remotes(root: &Utf8Path) -> Vec<(String, String)> {
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
pub(crate) fn ivar_on_github(fake: &FakeGh, rewrites: &[(String, String)]) -> assert_cmd::Command {
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
pub(crate) fn preview_on_github(
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
pub(crate) fn deliver_on_github(
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

/// Two promoted repos, both with something to deliver, so the batch has
/// siblings to link.
pub(crate) fn setup_two_repo_hall(root: &Utf8PathBuf) {
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

/// Point the declared repo's URL at `https://github.com/…` but redirect it,
/// via git's own `insteadOf`, to a path that does not exist — an immediate
/// local failure, so this never touches the network. Used to simulate the
/// remote not answering.
pub(crate) fn as_unreachable_github_remote(root: &Utf8Path) -> Vec<(String, String)> {
    let path = root.join("ivar.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut rewrites = Vec::new();
    for repo in value["repos"].as_array_mut().unwrap() {
        let name = repo["name"].as_str().unwrap().to_owned();
        let url = format!("https://github.com/acme/{name}.git");
        let broken = root.join("no-such-origin");
        rewrites.push((format!("url.{broken}.insteadOf"), url.clone()));
        repo["url"] = serde_json::Value::String(url);
    }
    std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
    rewrites
}

/// Deliver `feature` through the fake `gh`, expecting warnings (exit `1`)
/// rather than a clean run. Returns the apply document.
pub(crate) fn deliver_on_github_expecting_warnings(
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
        .code(1)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

/// Helper: preview on GitHub with extra args.
pub(crate) fn preview_on_github_with(
    root: &Utf8PathBuf,
    fake: &FakeGh,
    rewrites: &[(String, String)],
    feature: &str,
    extra: &[&str],
) -> serde_json::Value {
    let mut args: Vec<&str> = vec!["feature", "deliver", feature, "--preview", "--json"];
    args.extend_from_slice(extra);
    let output = ivar_on_github(fake, rewrites)
        .current_dir(root)
        .args(&args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

/// Helper: deliver on GitHub with extra args (using fingerprint from a fresh preview).
pub(crate) fn deliver_on_github_with(
    root: &Utf8PathBuf,
    fake: &FakeGh,
    rewrites: &[(String, String)],
    feature: &str,
    extra: &[&str],
) -> serde_json::Value {
    let preview = preview_on_github_with(root, fake, rewrites, feature, extra);
    let fp = preview["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut args: Vec<String> = vec![
        "feature".to_owned(),
        "deliver".to_owned(),
        feature.to_owned(),
        "--fingerprint".to_owned(),
        fp,
    ];
    for e in extra {
        args.push(e.to_string());
    }
    args.push("--json".to_owned());
    let output = ivar_on_github(fake, rewrites)
        .current_dir(root)
        .args(&args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

/// Set up a hall with one promoted repo, synced, feature created, repo promoted,
/// and one commit on the feature branch. Returns (guard, root).
pub(crate) fn setup_deliver_hall(root: &Utf8PathBuf) {
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

/// A hall with one promoted repo whose feature declares `develop` as its
/// base: `main`, plus `develop` carrying its own commit, merged into `main`
/// first when `merge_develop_into_main` is set — so the clone captures
/// whichever ancestry the caller needs before `develop` is (maybe) deleted
/// or advanced later in the test. Returns the origin path.
pub(crate) fn setup_deliver_hall_with_base(
    root: &Utf8PathBuf,
    merge_develop_into_main: bool,
) -> Utf8PathBuf {
    ivar().current_dir(root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    git(&origin, &["add", "develop-only.txt"]);
    git(&origin, &["commit", "-m", "develop work"]);
    git(&origin, &["checkout", "main"]);
    if merge_develop_into_main {
        git(
            &origin,
            &["merge", "--no-ff", "-m", "merge develop", "develop"],
        );
    }

    declare_repos(root, &[("api", &origin, "main")]);
    ivar().current_dir(root).arg("sync").assert().success();

    ivar()
        .current_dir(root)
        .args(["feature", "create", "checkout", "--base", "develop"])
        .assert()
        .success();
    ivar()
        .current_dir(root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    origin
}
