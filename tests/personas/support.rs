//! Helpers every persona scenario shares.

use camino::{Utf8Path, Utf8PathBuf};

use crate::common::{declare_repos, git, ivar, seeded_repo};

/// The compiled binary with nothing of the developer's environment: no
/// `IVAR_*` session, no provider or GitHub credentials, HOME and XDG inside
/// `home`.
pub(crate) fn isolated_ivar(home: &Utf8Path) -> assert_cmd::Command {
    let mut cmd = ivar();
    for (key, _) in std::env::vars() {
        if key.starts_with("IVAR_")
            || key.starts_with("CLAUDE")
            || key.starts_with("GIT_")
            || key == "GH_TOKEN"
            || key == "GITHUB_TOKEN"
        {
            cmd.env_remove(key);
        }
    }
    // The isolated HOME hides the user's global git identity, which ivar's own
    // integrate commits rely on, so the identity is provided here instead.
    cmd.env("IVAR_NO_UPDATE_CHECK", "1")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "ivar persona")
        .env("GIT_AUTHOR_EMAIL", "persona@ivar.invalid")
        .env("GIT_COMMITTER_NAME", "ivar persona")
        .env("GIT_COMMITTER_EMAIL", "persona@ivar.invalid");
    cmd
}

/// The isolated HOME for the hall at `root`, beside it in the same tempdir.
pub(crate) fn home(root: &Utf8Path) -> Utf8PathBuf {
    root.parent().unwrap().join("home")
}

/// Run `ivar <args> --json` in `root` and return the JSON, asserting `ok`.
///
/// The exit code is not asserted: a command that succeeds with warnings
/// exits 1 by contract.
pub(crate) fn run_ok(root: &Utf8Path, args: &[&str]) -> serde_json::Value {
    let output = isolated_ivar(&home(root))
        .current_dir(root)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "`ivar {}` printed no JSON ({e}): {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(
        value["ok"],
        true,
        "`ivar {}` failed: {value}",
        args.join(" ")
    );
    value
}

/// Initialise a hall at `root` declaring one seeded `main` repo per name, and
/// sync it. Returns each repo's origin, in `names` order.
pub(crate) fn hall_with_repos(root: &Utf8Path, names: &[&str]) -> Vec<Utf8PathBuf> {
    run_ok(root, &["init"]);
    let origins: Vec<Utf8PathBuf> = names
        .iter()
        .map(|name| seeded_repo(&root.parent().unwrap().join("origins").join(name), "main"))
        .collect();
    let repos: Vec<(&str, &Utf8Path, &str)> = names
        .iter()
        .zip(&origins)
        .map(|(name, origin)| (*name, origin.as_path(), "main"))
        .collect();
    declare_repos(root, &repos);
    run_ok(root, &["sync"]);
    origins
}

/// A synced hall with the single repo `app`. Returns its origin.
pub(crate) fn one_repo_hall(root: &Utf8Path) -> Utf8PathBuf {
    hall_with_repos(root, &["app"]).remove(0)
}

/// Commit a new `file` in `feature`'s worktree of `repo`.
pub(crate) fn commit_slice(root: &Utf8Path, repo: &str, feature: &str, file: &str) {
    let worktree = root.join(".ivar/repos").join(repo).join(feature);
    std::fs::write(worktree.join(file), format!("{feature}\n")).unwrap();
    git(&worktree, &["add", file]);
    git(&worktree, &["commit", "-m", feature]);
}
