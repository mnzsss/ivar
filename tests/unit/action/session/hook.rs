#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::Ctx;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// Two repos, one promoted. The unpromoted one is what proves the hook
/// does not run where the worktree is read-only.
fn hall_with_one_promoted_repo() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origins = root.parent().unwrap().join("origins");
    let layout = Layout::at(root.clone());
    let repos = ["api", "web"]
        .into_iter()
        .map(|name| {
            let origin = seeded_repo(&origins.join(name), "main");
            Repo::new(
                RepoName::new(name).unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )
        })
        .collect();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    (guard, root)
}

fn write_hook(root: &Utf8PathBuf, repo: &str, body: &str) {
    let hook = Layout::at(root.clone()).session_hook(&RepoName::new(repo).unwrap());
    fs::ensure_dir(hook.parent().unwrap()).unwrap();
    fs::write_text(&hook, body).unwrap();
}

fn run(root: &Utf8PathBuf) -> (Vec<Warning>, Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session);
    fs::ensure_dir(&view_dir).unwrap();

    let warnings = run_session_hooks(&layout, &manifest, &feature, &view_dir, &session);
    (warnings, view_dir)
}

/// The point of the whole file: a hook sees the session, which the setup
/// script cannot.
#[test]
fn a_session_hook_runs_in_the_worktree_with_the_session_environment() {
    let (_guard, root) = hall_with_one_promoted_repo();
    write_hook(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\n\
         printf '%s %s %s\\n%s\\n' \"$IVAR_REPO\" \"$IVAR_FEATURE\" \
         \"$IVAR_WORKTREE_KIND\" \"$IVAR_SESSION_ID\" > .ivar-hook-ran\n",
    );

    let (warnings, _view_dir) = run(&root);

    assert!(warnings.is_empty(), "was: {warnings:?}");
    let evidence =
        std::fs::read_to_string(root.join(".ivar/repos/api/checkout/.ivar-hook-ran")).unwrap();
    assert_eq!(
        evidence,
        "api checkout feature\n2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c\n"
    );
}

/// `IVAR_SESSION_PATH` is the view dir, so a hook can write beside the
/// session rather than into the repo it is bootstrapping.
#[test]
fn a_session_hook_gets_the_view_dir_as_the_session_path() {
    let (_guard, root) = hall_with_one_promoted_repo();
    write_hook(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf %s \"$IVAR_SESSION_PATH\" > \"$IVAR_SESSION_PATH/.ivar-hook-path\"\n",
    );

    let (warnings, view_dir) = run(&root);

    assert!(warnings.is_empty(), "was: {warnings:?}");
    assert_eq!(
        std::fs::read_to_string(view_dir.join(".ivar-hook-path")).unwrap(),
        view_dir.as_str()
    );
}

/// The common case, and it must cost nothing and say nothing.
#[test]
fn a_repo_without_a_session_hook_is_a_silent_no_op() {
    let (_guard, root) = hall_with_one_promoted_repo();

    let (warnings, _view_dir) = run(&root);

    assert!(warnings.is_empty(), "was: {warnings:?}");
}

/// A read-only repo has nothing for a hook to do, and running one would
/// fail against cleared write bits.
#[test]
fn an_unpromoted_repos_hook_does_not_run() {
    let (_guard, root) = hall_with_one_promoted_repo();
    write_hook(
        &root,
        "web",
        "#!/usr/bin/env bash\ntouch /tmp/ivar-web-hook-must-not-run\n",
    );

    let (warnings, _view_dir) = run(&root);

    assert!(warnings.is_empty(), "was: {warnings:?}");
    assert!(!std::path::Path::new("/tmp/ivar-web-hook-must-not-run").exists());
}

/// The view dir already exists and the agent is about to spawn. A failed
/// hook must not take the session down with it.
#[test]
fn a_failing_hook_warns_and_the_session_still_opens() {
    let (_guard, root) = hall_with_one_promoted_repo();
    write_hook(&root, "api", "#!/usr/bin/env bash\nexit 3\n");

    let (warnings, _view_dir) = run(&root);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "session.hook_failed");
    assert!(warnings[0].what.contains("exited 3"), "was: {warnings:?}");
}
