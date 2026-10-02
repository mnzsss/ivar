//! What a maintainer checks on a release candidate, for every provider.

use camino::Utf8PathBuf;

use super::support::{commit_slice, home, isolated_ivar, run_ok};
use crate::common::{git, hall_root, seeded_repo};

#[test]
fn only_a_build_from_its_own_checkout_reports_a_dev_version() {
    let (_guard, root) = hall_root();
    let output = isolated_ivar(&home(&root))
        .arg("--version")
        .output()
        .unwrap();
    let version = String::from_utf8(output.stdout).unwrap();
    if option_env!("IVAR_RELEASE").is_some() || !built_from_own_checkout() {
        assert!(
            !version.contains("-dev"),
            "release build looks local: {version}"
        );
    } else {
        assert!(
            version.contains("-dev+"),
            "local build looks like a release: {version}"
        );
    }
}

#[test]
fn the_maintainer_release_smoke_passes_for_every_provider() {
    for provider in ["claude-code", "opencode", "omp"] {
        let (_guard, root) = hall_root();
        run_ok(&root, &["init", "--provider", provider]);
        let origin = seeded_repo(&root.parent().unwrap().join("origins/app"), "main");
        std::fs::write(origin.join(".gitignore"), "setup-ran\n").unwrap();
        git(&origin, &["add", ".gitignore"]);
        git(&origin, &["commit", "-m", "ignore setup output"]);
        run_ok(&root, &["repo", "add", "app", &format!("file://{origin}")]);
        std::fs::create_dir_all(root.join(".ivar/setups")).unwrap();
        std::fs::write(root.join(".ivar/setups/app.sh"), "touch setup-ran\n").unwrap();
        run_ok(&root, &["sync"]);

        run_ok(&root, &["feature", "create", "release"]);
        let promoted = run_ok(&root, &["feature", "promote", "release", "app"]);
        assert_eq!(promoted["setup_ran"], true, "{provider}: {promoted}");
        assert!(
            root.join(".ivar/repos/app/release/setup-ran").is_file(),
            "{provider}: setup script did not run in the worktree"
        );

        run_ok(&root, &["feature", "create", "fix", "--parent", "release"]);
        run_ok(&root, &["feature", "promote", "fix", "app"]);
        commit_slice(&root, "app", "fix", "fix.txt");
        run_ok(&root, &["plan", "create", "fix", "plan"]);
        run_ok(&root, &["plan", "approve", "fix", "plan"]);
        let integrated = run_ok(&root, &["feature", "integrate", "fix"]);
        assert_eq!(
            integrated["state"], "integrated",
            "{provider}: {integrated}"
        );

        let preview = run_ok(&root, &["feature", "deliver", "release", "--preview"]);
        assert_eq!(
            preview["preview"]["repos"].as_array().map(Vec::len),
            Some(1),
            "{provider}: {preview}"
        );

        let session = run_ok(&root, &["session", "start", "release", "--detached"]);
        assert_eq!(session["provider"], provider, "{session}");
        let view_dir = Utf8PathBuf::from(session["view_dir"].as_str().unwrap());
        assert!(view_dir.is_dir(), "{provider}: no view dir");
        let stopped = run_ok(&root, &["session", "stop"]);
        assert_eq!(stopped["stopped"], 1, "{provider}: {stopped}");
        assert!(!view_dir.exists(), "{provider}: view dir survived stop");

        let doctor = run_ok(&root, &["doctor"]);
        assert_eq!(
            doctor["findings"],
            serde_json::json!([]),
            "{provider}: {doctor}"
        );
    }
}

/// Whether the crate under test is the toplevel of a git checkout, which is
/// when `build.rs` marks the binary as a dev build.
fn built_from_own_checkout() -> bool {
    let manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&manifest_dir)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|toplevel| Utf8PathBuf::from(toplevel.trim()).canonicalize_utf8().ok())
        .is_some_and(|toplevel| Some(toplevel) == manifest_dir.canonicalize_utf8().ok())
}
