//! End-to-end tests for `ivar repo create`, driving the compiled binary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use common::{FakeGh, hall_root, ivar};

#[test]
fn create_remote_makes_a_private_github_repo_pushes_a_readme_and_registers_it() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme"])
        .assert()
        .success();
    let fake = FakeGh::install(&root);
    let github = root.parent().unwrap().join("github");
    std::fs::create_dir_all(&github).unwrap();

    let out = ivar()
        .current_dir(&root)
        .env(
            "PATH",
            format!("{}:{}", fake.dir, std::env::var("PATH").unwrap()),
        )
        .env("GH_FAKE_LOG", fake.log.as_str())
        .env("GH_FAKE_STATE", fake.state.as_str())
        .env("GH_FAKE_CHECKS", fake.checks.as_str())
        .env("FAKE_GH_LOGIN", "acme")
        .env("FAKE_GH_REPOS", github.as_str())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", format!("url.{github}/.insteadOf"))
        .env("GIT_CONFIG_VALUE_0", "https://github.com/")
        .args(["repo", "create", "notes", "--remote", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["url"], "https://github.com/acme/notes");
    assert!(json["ref_prefix"].is_null());
    assert!(fake.log().contains("repo create acme/notes --private"));
    assert!(root.join(".ivar/repos/notes/main/README.md").is_file());
}

#[test]
fn create_remote_refuses_an_existing_github_repo_before_any_write() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme"])
        .assert()
        .success();
    let fake = FakeGh::install(&root);
    let github = root.parent().unwrap().join("github");
    std::fs::create_dir_all(github.join("acme/notes")).unwrap();

    ivar()
        .current_dir(&root)
        .env(
            "PATH",
            format!("{}:{}", fake.dir, std::env::var("PATH").unwrap()),
        )
        .env("GH_FAKE_LOG", fake.log.as_str())
        .env("GH_FAKE_STATE", fake.state.as_str())
        .env("GH_FAKE_CHECKS", fake.checks.as_str())
        .env("FAKE_GH_LOGIN", "acme")
        .env("FAKE_GH_REPOS", github.as_str())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["repo", "create", "notes", "--remote", "--json"])
        .assert()
        .failure()
        .code(2);

    assert!(!fake.log().contains("repo create"));
    assert!(!root.join(".ivar/repos/notes").exists());
}
