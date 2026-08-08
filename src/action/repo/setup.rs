//! `ivar repo setup <repo>` — run one repo's setup script in isolation.
//!
//! `ivar sync` runs every repo's setup script when it needs running; this
//! verb does the same for exactly one repo, through the same function
//! ([`crate::action::sync::run_setup_script`]), so the two paths share the
//! receipt logic and cannot drift.
//!
//! The receipt is respected: a script whose content has not changed since its
//! last run is reported as already-run and not executed again. `--force-setup`
//! ignores the receipt and runs it anyway — the same flag `ivar sync` honours.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;
use crate::action::sync::{self, Change};

/// What `ivar repo setup` needs.
#[derive(Debug, Clone)]
pub struct SetupInput {
    /// The repo whose setup script to run, as declared in `ivar.json`.
    pub repo: String,
    /// Ignore the receipt and run the setup script even if its content has
    /// not changed since the last run.
    pub force: bool,
}

/// What `ivar repo setup` did.
#[derive(Debug, Clone, Serialize)]
pub struct SetupOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo whose setup script was (or was not) run.
    pub repo: RepoName,
    /// The script's expected location, `.ivar/setups/<repo>.sh`.
    pub script: Utf8PathBuf,
    /// What happened to the setup state. `None` when the repo has no script —
    /// the explained no-op.
    pub change: Option<Change>,
    /// Anything worth saying beyond the change — why the script was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WriteHuman for SetupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self.change {
            None => writeln!(
                w,
                "No setup script for `{}` at `{}` — nothing to run.",
                self.repo, self.script
            ),
            Some(Change::Created) => writeln!(w, "Ran setup script for `{}`.", self.repo),
            Some(Change::Updated) => writeln!(w, "Re-ran setup script for `{}`.", self.repo),
            Some(Change::Unchanged) => match &self.detail {
                Some(detail) => {
                    writeln!(w, "Setup script for `{}` not run — {detail}.", self.repo)
                }
                None => writeln!(w, "Setup script for `{}` not run.", self.repo),
            },
            Some(other) => writeln!(w, "Setup script for `{}`: {:?}", self.repo, other),
        }
    }
}

/// Run `input.repo`'s setup script in its default worktree, if it has one.
///
/// Blocked when the repo is not registered in `ivar.json`, or when its
/// default-branch worktree does not exist (nothing for the script to run in —
/// `ivar sync` materialises worktrees). A repo without a script is an
/// explained no-op, not an error.
pub fn setup(ctx: &Ctx, input: SetupInput) -> Outcome<SetupOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = RepoName::new(input.repo)?;
    let repo = manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == &name)
        .ok_or_else(|| {
            Failure::blocked(
                "repo.setup_repo_not_found",
                format!("repo `{name}` is not in ivar.json"),
            )
            .expected("a repo declared in `ivar.json`")
            .actual(format!("`{name}` is not among the declared repos"))
            .fix(FixAction::safe(
                "repo.add_first",
                format!("Add it first with `ivar repo add {name}`, then run setup again."),
            ))
        })?;

    let script = layout.setup_script(&name);

    // A repo without a script is an explained no-op — nothing to run, and no
    // worktree requirement either.
    if !fs::is_file(&script)? {
        return Ok(Report::new(SetupOutcome {
            root: layout.root().to_path_buf(),
            repo: name,
            script,
            change: None,
            detail: None,
        }));
    }

    let worktree = layout.repo_worktree(&name, repo.default_branch());
    match git.target_state(&worktree)? {
        TargetState::Repository => {}
        _ => {
            return Err(Failure::blocked(
                "repo.setup_worktree_missing",
                format!("`{worktree}` is not a materialised worktree for `{name}`"),
            )
            .expected("the repo's default-branch worktree to exist")
            .actual("it is missing, or is not a git worktree")
            .fix(
                FixAction::safe(
                    "repo.sync_first",
                    "Run `ivar sync` to materialise the worktree, then run setup again.",
                )
                .command("ivar sync"),
            ));
        }
    }

    let surface = format!("repo {name}");
    let (change, detail) =
        match sync::run_setup_script(&git, &layout, repo, &worktree, &surface, input.force)? {
            None => (None, None),
            Some(entry) => (Some(entry.change), entry.detail),
        };
    Ok(Report::new(SetupOutcome {
        root: layout.root().to_path_buf(),
        repo: name,
        script,
        change,
        detail,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
}
