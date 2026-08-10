//! The per-repo **Session Hook** — `.ivar/setups/<repo>.session.sh`, run once
//! per `session start`, in each promoted repo's feature worktree.
//!
//! # Why this is not the setup script
//!
//! `.ivar/setups/<repo>.sh` bootstraps a *worktree*: it installs dependencies
//! and materialises `.env`, and a receipt in the worktree's git admin directory
//! makes sure it runs about once. That receipt is the whole reason the setup
//! script is cheap enough that people keep running `ivar sync` — see
//! [`crate::store::setup_receipt`].
//!
//! Per-session state cannot live there. A session's database or compose project
//! has to be brought up *every* time a session opens, and several sessions can
//! share one promoted worktree, so a receipt keyed to the worktree would skip
//! exactly the runs that matter. The two lifetimes are different, so they get
//! two files.
//!
//! | | setup script | session hook |
//! | --- | --- | --- |
//! | file | `.ivar/setups/<repo>.sh` | `.ivar/setups/<repo>.session.sh` |
//! | runs on | `sync`, `promote`, `repo setup` | `session start` |
//! | how often | once per worktree, receipt-gated | once per session, ungated |
//! | typical body | `pnpm install`, `cp .env.example .env` | `docker compose up -d` |
//!
//! # Failure is a warning, never a refusal
//!
//! `promote` already treats a failed setup script as non-fatal: the repo stays
//! promoted and the user gets a warning. The same reasoning applies here with
//! more force — the view dir exists, the agent is about to spawn, and refusing
//! to open a session because one repo's optional hook exited non-zero would
//! trade a working session for no session at all. Every hook is attempted even
//! after one fails, for the same reason `sync` keeps going.
//!
//! # The environment
//!
//! Everything the setup script gets on the promote path, plus the two session
//! variables that have no meaning outside a session:
//!
//! - `IVAR_SESSION_ID` — this session's id.
//! - `IVAR_SESSION_PATH` — the view dir.
//!
//! `ARCHITECTURE.md` lists both in the environment contract. This is the file
//! that makes that true for a script.

use crate::domain::feature::Feature;
use crate::domain::name::{RepoName, SessionId};
use crate::error::Warning;
use crate::infra::{fs, proc};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// The interpreter a session hook runs under — `bash`, the same choice the
/// setup script makes, and for the same reason: a `.sh` arriving through a
/// clone may lack its executable bit.
const HOOK_INTERPRETER: &str = "bash";

/// Run every promoted repo's session hook, in manifest order.
///
/// A repo without a hook is skipped silently — the common case. A repo that is
/// not promoted is skipped too: its worktree is held read-only for this
/// session, and a hook that cannot write is a hook with nothing to do.
///
/// Returns one [`Warning`] per hook that failed. An empty vector is the happy
/// path and the usual one.
pub(crate) fn run_session_hooks(
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    view_dir: &camino::Utf8Path,
    session: &SessionId,
) -> Vec<Warning> {
    manifest
        .repos()
        .iter()
        .filter(|repo| feature.is_promoted(repo.name()))
        .filter_map(|repo| run_hook(layout, repo.name(), feature, view_dir, session).err())
        .collect()
}

/// Run one repo's hook. `Ok(())` covers both "there is no hook" and "it ran and
/// exited zero" — from the caller's side those are the same outcome, and the
/// only thing worth returning is the warning when there is one.
fn run_hook(
    layout: &Layout,
    repo: &RepoName,
    feature: &Feature,
    view_dir: &camino::Utf8Path,
    session: &SessionId,
) -> Result<(), Warning> {
    let hook = layout.session_hook(repo);
    match fs::is_file(&hook) {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(error) => {
            return Err(Warning::new(
                "session.hook_unreadable",
                repo.to_string(),
                error.to_string(),
            ));
        }
    }

    // The hook runs in the worktree, not the view dir: a `docker compose` file
    // lives in the repo, and a hook that had to `cd` its way there would be
    // guessing at a path this module already knows.
    let worktree = layout.repo_worktree(repo, &feature.branch);
    match fs::is_dir(&worktree) {
        Ok(true) => {}
        Ok(false) => {
            return Err(Warning::new(
                "session.hook_no_worktree",
                repo.to_string(),
                format!("`{worktree}` does not exist — run `ivar sync`"),
            ));
        }
        Err(error) => {
            return Err(Warning::new(
                "session.hook_no_worktree",
                repo.to_string(),
                error.to_string(),
            ));
        }
    }

    let command = proc::Command::new(HOOK_INTERPRETER)
        .arg(hook.as_str())
        .cwd(&worktree)
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo.as_str())
        .env("IVAR_BRANCH", feature.branch.as_str())
        .env("IVAR_WORKTREE", worktree.as_str())
        .env("IVAR_WORKTREE_KIND", "feature")
        .env("IVAR_FEATURE", feature.name.as_str())
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
        .env("IVAR_SESSION_ID", session.as_str())
        .env("IVAR_SESSION_PATH", view_dir.as_str());

    // Streamed, not captured, for the same reason the setup script is: a
    // `docker compose up` pulling an image is minutes of output, and a frozen
    // line is indistinguishable from a hang.
    match proc::inherit(&command) {
        Ok(Some(0)) => Ok(()),
        Ok(code) => Err(Warning::new(
            "session.hook_failed",
            repo.to_string(),
            format!("`{hook}` {}", ended(code)),
        )),
        Err(error) => Err(Warning::new(
            "session.hook_failed",
            repo.to_string(),
            error.to_string(),
        )),
    }
}

/// How a process ended, in the words `sync` and `promote` already use for the
/// setup script. Same failure, same sentence.
fn ended(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exited {code}"),
        None => "was killed by a signal".to_owned(),
    }
}

#[cfg(test)]
mod tests {
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
}
