//! `ivar repo upstream <repo> <url>` — set the `upstream` remote of a
//! registered repo's bare clone.
//!
//! A fork workflow tracks the original repository as a remote named
//! `upstream`, alongside the manifest's `origin` URL. This verb manages that
//! remote on the repo's bare clone under `.ivar/` — it does not touch the
//! manifest, which owns only the repo's `url` (the fork).
//!
//! Invalid upstreams are refused **before** anything is written: a blank URL
//! never reaches git, so the bare clone's config is untouched by a bad
//! invocation. An existing `upstream` remote is re-pointed; an absent one is
//! added.
//!
//! git's own mutation runs through `infra::proc` (the same boundary
//! `sync`'s setup runner uses) because the `Git` trait does not expose remote
//! management — adding that would touch `git/mod.rs`, which is not this
//! verb's to change.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::proc;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// The remote name this verb manages. Fixed — a repo's upstream remote is
/// called `upstream` by every tool that knows the concept.
const REMOTE: &str = "upstream";

/// What `ivar repo upstream` needs.
#[derive(Debug, Clone)]
pub struct UpstreamInput {
    /// The repo whose upstream remote to manage, as declared in `ivar.json`.
    pub repo: String,
    /// The upstream remote URL. A blank URL is refused before writing —
    /// unless `remove` is set, which deletes the remote instead.
    pub url: String,
    /// Remove the upstream remote entirely. Mutually exclusive with `url`.
    pub remove: bool,
}

/// What `ivar repo upstream` did.
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo whose remote was changed.
    pub repo: RepoName,
    /// The remote that was changed — always `upstream`.
    pub remote: String,
    /// The URL the remote now points at.
    pub url: String,
    /// Whether the remote was added (true) or re-pointed (false).
    pub added: bool,
}

impl WriteHuman for UpstreamOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let verb = if self.added { "Added" } else { "Updated" };
        writeln!(
            w,
            "{verb} `{}` remote for `{}` in {} ← {}",
            self.remote, self.repo, self.root, self.url
        )
    }
}

/// Set `input.repo`'s `upstream` remote to `input.url`.
///
/// Blocked when the repo is not registered in `ivar.json`, the URL is blank,
/// or the repo's bare clone does not exist (there is nowhere for the remote
/// to live — `ivar sync` materialises clones). Failed when git refuses the
/// remote operation itself.
pub fn upstream(ctx: &Ctx, input: UpstreamInput) -> Outcome<UpstreamOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let name = RepoName::new(input.repo)?;
    manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == &name)
        .ok_or_else(|| {
            Failure::blocked(
                "repo.upstream_repo_not_found",
                format!("repo `{name}` is not in ivar.json"),
            )
            .expected("a repo declared in `ivar.json`")
            .actual(format!("`{name}` is not among the declared repos"))
            .fix(FixAction::safe(
                "repo.add_first",
                format!("Add it first with `ivar repo add {name}`."),
            ))
        })?;

    // Refused before anything is written: a blank upstream never reaches git,
    // so the bare clone's config stays untouched. Removal is the one path a
    // blank URL is valid — it is the whole point of `--remove`.
    let remove = input.remove;
    let url = input.url;
    if !remove && url.trim().is_empty() {
        return Err(Failure::blocked(
            "repo.upstream_invalid_url",
            "a blank upstream URL is not a remote",
        )
        .expected("a non-blank git remote URL")
        .actual("an empty (or whitespace-only) URL")
        .fix(FixAction::safe(
            "repo.upstream_pass_url",
            "Pass the upstream URL explicitly, e.g. `ivar repo upstream <repo> git@github.com:owner/repo.git`.",
        )));
    }

    let bare = layout.repo_bare(&name);
    match git::System.target_state(&bare)? {
        TargetState::Repository => {}
        _ => {
            return Err(Failure::blocked(
                "repo.upstream_bare_missing",
                format!("`{bare}` is not a materialised bare clone for `{name}`"),
            )
            .expected("the repo's bare clone to exist under `.ivar/`")
            .actual("it is missing, or is not a git repository")
            .fix(
                FixAction::safe(
                    "repo.sync_first",
                    "Run `ivar sync` to materialise the clone, then set the upstream again.",
                )
                .command("ivar sync"),
            ));
        }
    }

    // `git remote get-url upstream` exiting non-zero is the answer "the
    // remote does not exist yet" — the exit code is data, not an error.
    let probe = proc::capture(&git_remote(&bare, &["get-url", REMOTE]))?;
    let added = !probe.success();

    if remove {
        if added {
            // Nothing to remove — the remote was never there. A no-op that
            // says so is more honest than an error.
            return Ok(Report::new(UpstreamOutcome {
                root: layout.root().to_path_buf(),
                repo: name,
                remote: REMOTE.to_owned(),
                url: String::new(),
                added: false,
            }));
        }
        let output = proc::capture(&git_remote(&bare, &["remove", REMOTE]))?;
        if !output.success() {
            return Err(Failure::failed(
                "repo.upstream_git_refused",
                format!("`git remote remove {REMOTE}` failed for `{name}`"),
            )
            .expected("git to drop the `upstream` remote")
            .actual(output.diagnostic())
            .fix(FixAction::safe(
                "repo.upstream_read_git_error",
                "Run the command shown above by hand — git's own message names what it needs.",
            )));
        }
        return Ok(Report::new(UpstreamOutcome {
            root: layout.root().to_path_buf(),
            repo: name,
            remote: REMOTE.to_owned(),
            url: String::new(),
            added: false,
        }));
    }

    let verb = if added { "add" } else { "set-url" };
    let output = proc::capture(&git_remote(&bare, &[verb, REMOTE]).arg(&url))?;
    if !output.success() {
        return Err(Failure::failed(
            "repo.upstream_git_refused",
            format!("`git remote {verb} {REMOTE}` failed for `{name}`"),
        )
        .expected("git to record the `upstream` remote")
        .actual(output.diagnostic())
        .fix(FixAction::safe(
            "repo.upstream_read_git_error",
            "Run the command shown above by hand — git's own message names what it needs.",
        )));
    }

    Ok(Report::new(UpstreamOutcome {
        root: layout.root().to_path_buf(),
        repo: name,
        remote: REMOTE.to_owned(),
        url,
        added,
    }))
}

/// `git --git-dir <bare> remote <args...>` — remote management runs against
/// the bare clone directly, so the remote lives in the same repository every
/// worktree and feature branch of the repo share.
fn git_remote(bare: &Utf8Path, args: &[&str]) -> proc::Command {
    proc::Command::new("git")
        .arg("--git-dir")
        .arg(bare.as_str())
        .arg("remote")
        .args(args.iter().copied())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    const UPSTREAM_URL: &str = "git@example.com:upstream/api.git";

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

    fn input(repo: &str, url: &str) -> UpstreamInput {
        UpstreamInput {
            repo: repo.to_owned(),
            url: url.to_owned(),
            remove: false,
        }
    }

    /// The bare clone's recorded `upstream` URL, or `None` when the remote
    /// does not exist.
    fn upstream_url(root: &Utf8PathBuf) -> Option<String> {
        let bare = Layout::at(root.clone()).repo_bare(&RepoName::new("api").unwrap());
        let output = proc::capture(&git_remote(&bare, &["get-url", REMOTE])).unwrap();
        output.success().then(|| output.stdout)
    }

    #[test]
    fn upstream_adds_the_remote_to_the_bare_clone() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());

        let report = upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();

        assert!(report.is_clean());
        assert!(report.value.added);
        assert_eq!(upstream_url(&root).as_deref(), Some(UPSTREAM_URL));
    }

    #[test]
    fn upstream_repoints_an_existing_remote() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();

        let report = upstream(&ctx, input("api", "git@example.com:other/api.git")).unwrap();

        assert!(!report.value.added);
        assert_eq!(
            upstream_url(&root).as_deref(),
            Some("git@example.com:other/api.git")
        );
    }

    /// The invalid-upstream guard: a blank URL is refused before anything is
    /// written, so the bare clone gains no `upstream` remote.
    #[test]
    fn upstream_refuses_a_blank_url_before_writing() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());

        let failure = upstream(&ctx, input("api", "   ")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.upstream_invalid_url");
        assert_eq!(upstream_url(&root), None, "nothing may be written");
    }

    #[test]
    fn upstream_remove_drops_the_remote() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());

        // Set the upstream first.
        upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();
        assert!(upstream_url(&root).is_some());

        // Then remove it.
        let report = upstream(
            &ctx,
            UpstreamInput {
                repo: "api".to_owned(),
                url: String::new(),
                remove: true,
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(upstream_url(&root), None, "the remote must be gone");
    }

    #[test]
    fn upstream_remove_of_a_missing_remote_is_a_no_op() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());

        let report = upstream(
            &ctx,
            UpstreamInput {
                repo: "api".to_owned(),
                url: String::new(),
                remove: true,
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(upstream_url(&root), None);
    }

    #[test]
    fn upstream_is_refused_for_a_repo_not_in_the_manifest() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root);

        let failure = upstream(&ctx, input("ghost", UPSTREAM_URL)).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.upstream_repo_not_found");
    }

    #[test]
    fn upstream_is_refused_when_the_clone_is_missing() {
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
        // Declared but never synced — no bare clone exists.

        let failure = upstream(&ctx, input("api", UPSTREAM_URL)).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.upstream_bare_missing");
        assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
        drop(guard);
    }

    #[test]
    fn the_human_surface_names_what_happened() {
        let outcome = UpstreamOutcome {
            root: Utf8PathBuf::from("/hall"),
            repo: RepoName::new("api").unwrap(),
            remote: REMOTE.to_owned(),
            url: UPSTREAM_URL.to_owned(),
            added: true,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Added `upstream` remote for `api` in /hall ← git@example.com:upstream/api.git\n"
        );
    }
}
