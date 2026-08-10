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
#[path = "../../../tests/unit/action/repo/upstream.rs"]
mod tests;
