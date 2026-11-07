//! Orphan worktrees: what is known about each one cheaply, and whether it can
//! go.
//!
//! `repo.worktree_orphaned` used to give every orphan the same advice —
//! inspect it — and leave the inspection to the user: dirty files, whether the
//! branch was pushed, whether its pull request merged, whether a squash-merge
//! already put the work on the default branch. This module does that
//! inspection and states the result, so the finding says which worktrees are
//! safe to remove and why the others are not.
//!
//! Everything but the pull-request lookup is local: git reads and the
//! remote-tracking refs as of the last fetch. The lookup is a network call
//! through `gh`, so it runs only when the user asked for it
//! (`ivar doctor --github`), and it goes through [`PullRequestLookup`] so tests
//! never reach a forge.

use super::features::is_read_only;
use super::*;
use crate::action::feature::pull_requests;

/// What `ivar doctor` was asked to do beyond reading the hall.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DoctorInput {
    /// Ask GitHub, through `gh`, what became of each orphan branch's pull
    /// request. Off by default: it is a network call.
    pub github: bool,
    /// Remove the orphan worktrees classified safe, and only those.
    pub fix: bool,
}

/// Whether a pull request is open, merged, or closed without merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestState {
    /// Still open (or queued, or any state that is not final).
    Open,
    /// Merged into its base.
    Merged,
    /// Closed without merging.
    Closed,
}

/// A pull request whose head is a given branch, as the doctor reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPullRequest {
    /// The host-assigned number.
    pub number: u64,
    /// Where it stands.
    pub state: PullRequestState,
    /// The head commit the forge recorded. `None` when it did not say.
    pub head_oid: Option<String>,
}

/// The one way the doctor asks a forge about a branch.
///
/// A trait because the answer is a network call: production reaches GitHub
/// through [`GhLookup`], and tests hand in a fake.
pub trait PullRequestLookup {
    /// Every pull request, in any state, whose head is `branch` in the
    /// repository whose bare clone is `bare`. The error is a sentence for the
    /// finding: `gh` missing, not signed in, offline.
    ///
    /// # Errors
    ///
    /// Returns the reason the forge could not be asked.
    fn for_branch(&self, bare: &Utf8Path, branch: &str) -> Result<Vec<BranchPullRequest>, String>;
}

/// [`PullRequestLookup`] through the `gh` executable.
#[derive(Debug, Clone, Copy, Default)]
pub struct GhLookup;

impl PullRequestLookup for GhLookup {
    fn for_branch(&self, bare: &Utf8Path, branch: &str) -> Result<Vec<BranchPullRequest>, String> {
        let found = pull_requests::list_pull_requests(bare, branch, "all")
            .map_err(|failure| failure.what)?;
        Ok(found
            .into_iter()
            .map(|pr| BranchPullRequest {
                number: pr.number,
                state: match pr.state.as_str() {
                    "MERGED" => PullRequestState::Merged,
                    "CLOSED" => PullRequestState::Closed,
                    _ => PullRequestState::Open,
                },
                head_oid: pr.head_oid,
            })
            .collect())
    }
}

/// What the doctor was asked to do about orphans, for one scan.
pub(super) struct OrphanScan<'a, P> {
    /// `Some` only when the user asked for the GitHub lookup.
    pub prs: Option<&'a P>,
    /// Remove the worktrees classified safe.
    pub fix: bool,
}

/// The verdict on one orphan worktree and the facts behind it.
#[derive(Debug)]
struct Classification {
    safe: bool,
    facts: Vec<String>,
}

pub(super) fn orphan_diagnosis<P: PullRequestLookup>(
    git: &impl Git,
    scan: &OrphanScan<'_, P>,
    site: &OrphanSite<'_>,
    removed: &mut Vec<Utf8PathBuf>,
) -> Option<Diagnosis> {
    let OrphanSite {
        bare,
        repo,
        default_branch,
        entry,
        branch,
        ..
    } = site;
    let path = &entry.path;
    let shown = branch.unwrap_or("detached");
    if entry.prunable {
        return Some(Diagnosis {
            code: "repo.worktree_orphaned",
            what: format!(
                "`{path}` in `{repo}` is on branch `{shown}`, which no feature owns — its directory is gone"
            ),
            fix: "Run `ivar sync` to drop the stale registration.".to_owned(),
        });
    }
    let class = classify(git, scan.prs, bare, default_branch, entry, *branch);
    if class.safe && scan.fix {
        match git.remove_worktree(bare, path) {
            Ok(()) => {
                removed.push(path.clone());
                return None;
            }
            Err(error) => {
                return Some(finding(
                    site,
                    &class,
                    &format!("`ivar doctor --fix` could not remove it: {error}"),
                ));
            }
        }
    }
    Some(finding(site, &class, ""))
}

/// Where an orphan sits and what is known about its place in the hall.
pub(super) struct OrphanSite<'a> {
    pub bare: &'a Utf8Path,
    pub repo: &'a str,
    pub default_branch: &'a str,
    pub entry: &'a WorktreeEntry,
    /// `None` for a detached worktree.
    pub branch: Option<&'a str>,
    /// The feature that was the owner of the folder, when the folder is named
    /// for a branch other than the one now checked out in it.
    pub folder_owner: Option<&'a FeatureName>,
}

fn finding(site: &OrphanSite<'_>, class: &Classification, failure: &str) -> Diagnosis {
    let path = &site.entry.path;
    let repo = site.repo;
    let shown = site.branch.unwrap_or("detached");
    let verdict = if class.safe {
        "safe to remove (integrated, clean)"
    } else {
        "needs a look (unsent or unmerged work)"
    };
    let owner = site
        .folder_owner
        .map(|feature| format!("; the folder was feature `{feature}`'s"))
        .unwrap_or_default();
    let remove = format!("git --git-dir {} worktree remove {path}", site.bare);
    // A checkout that was the default or a base branch carries ivar's
    // read-only guard on its root, and git cannot delete the entries of a
    // directory it cannot write to: it drops the registration, then aborts,
    // leaving the files behind unregistered. Lift the guard first.
    let remove = if is_read_only(path) {
        format!("chmod u+w {path} && {remove}")
    } else {
        remove
    };
    let fix = if class.safe {
        format!("Remove it with `{remove}`, or run `ivar doctor --fix`.")
    } else {
        format!("Inspect it; if nothing is needed, run `{remove}`.")
    };
    let fix = if failure.is_empty() {
        fix
    } else {
        format!("{failure}. {fix}")
    };
    Diagnosis {
        code: "repo.worktree_orphaned",
        what: format!(
            "`{path}` in `{repo}` is on branch `{shown}`, which no feature owns — {verdict}: {}{owner}",
            class.facts.join("; ")
        ),
        fix,
    }
}

/// Read what is cheap to know about the orphan at `entry.path` and decide.
///
/// Safe means three things at once: nothing uncommitted, nothing commit-shaped
/// not yet pushed to the branch's upstream, and the branch's work already on the
/// default branch — shown by a merged pull request at exactly this commit, or
/// locally, by every file the branch changed already matching the default
/// branch. A fact that cannot be read is never read as good news.
fn classify<P: PullRequestLookup>(
    git: &impl Git,
    prs: Option<&P>,
    bare: &Utf8Path,
    default_branch: &str,
    entry: &WorktreeEntry,
    branch: Option<&str>,
) -> Classification {
    let path = &entry.path;
    let mut safe = true;
    let mut facts = Vec::new();

    match git.changed_paths(path) {
        Ok(changed) if changed.is_empty() => facts.push("no uncommitted changes".to_owned()),
        Ok(changed) => {
            safe = false;
            facts.push(format!(
                "{} uncommitted {}",
                changed.len(),
                pluralize(changed.len(), "change")
            ));
        }
        Err(error) => {
            safe = false;
            facts.push(format!("could not read its status ({error})"));
        }
    }

    let head = match git.head_commit(path) {
        Ok(head) => head,
        Err(error) => {
            facts.push(format!("could not read its HEAD ({error})"));
            return Classification { safe: false, facts };
        }
    };

    if let Some(branch) = branch {
        remote_facts(git, bare, branch, &head, &mut safe, &mut facts);
    }

    let pr_merged = match (prs, branch) {
        (Some(prs), Some(branch)) => pull_request_fact(prs, bare, branch, &head, &mut facts),
        _ => false,
    };
    let integrated = default_branch_fact(git, bare, path, default_branch, &head, &mut facts);
    if !(pr_merged || integrated) {
        safe = false;
    }
    Classification { safe, facts }
}

/// Whether the branch is on the remote and how far it is ahead of it. Reads
/// the remote-tracking ref as of the last fetch; it makes no network call.
fn remote_facts(
    git: &impl Git,
    bare: &Utf8Path,
    branch: &str,
    head: &str,
    safe: &mut bool,
    facts: &mut Vec<String>,
) {
    let tracking = format!("refs/remotes/origin/{branch}");
    if git.revision_commit(bare, &tracking).is_err() {
        facts.push("no remote branch (deleted or never pushed)".to_owned());
        return;
    }
    match git.commits_ahead(bare, &tracking, head) {
        Ok(0) => facts.push(format!("everything is pushed to `origin/{branch}`")),
        Ok(ahead) => {
            *safe = false;
            facts.push(format!(
                "{ahead} {} not pushed to `origin/{branch}`",
                pluralize(usize::try_from(ahead).unwrap_or(usize::MAX), "commit")
            ));
        }
        Err(error) => {
            *safe = false;
            facts.push(format!(
                "could not compare with `origin/{branch}` ({error})"
            ));
        }
    }
}

/// Ask the forge about the branch and say what came back. `true` only when a
/// pull request merged at exactly `head`: a PR merged at an earlier commit
/// says nothing about later work.
fn pull_request_fact(
    prs: &impl PullRequestLookup,
    bare: &Utf8Path,
    branch: &str,
    head: &str,
    facts: &mut Vec<String>,
) -> bool {
    let found = match prs.for_branch(bare, branch) {
        Ok(found) => found,
        Err(reason) => {
            facts.push(format!(
                "pull request unknown ({}); compared files locally",
                reason.trim()
            ));
            return false;
        }
    };
    if let Some(pr) = found
        .iter()
        .find(|pr| pr.state == PullRequestState::Merged && pr.head_oid.as_deref() == Some(head))
    {
        facts.push(format!("PR #{} merged", pr.number));
        return true;
    }
    if let Some(pr) = found.iter().find(|pr| pr.state == PullRequestState::Merged) {
        facts.push(format!("PR #{} merged at a different commit", pr.number));
    } else if let Some(pr) = found.first() {
        let state = match pr.state {
            PullRequestState::Closed => "closed without merging",
            PullRequestState::Open | PullRequestState::Merged => "still open",
        };
        facts.push(format!("PR #{} {state}", pr.number));
    } else {
        facts.push("no pull request for the branch".to_owned());
    }
    false
}

/// Whether the work at `head` is already on the default branch, judged from
/// local refs alone.
///
/// A squash-merge leaves the branch's commits outside the default branch's
/// history even though the work landed, so a commit count is not enough: when
/// commits are ahead, every file the branch changed since it forked is compared
/// with the default branch's copy. A file the default branch changed again
/// afterwards differs, and the worktree then needs a look — the safe error.
fn default_branch_fact(
    git: &impl Git,
    bare: &Utf8Path,
    path: &Utf8Path,
    default_branch: &str,
    head: &str,
    facts: &mut Vec<String>,
) -> bool {
    let remote = format!("refs/remotes/origin/{default_branch}");
    let base = if git.revision_commit(bare, &remote).is_ok() {
        remote
    } else {
        default_branch.to_owned()
    };
    let name = format!("`{default_branch}`");
    match compare_with_base(git, bare, path, &base, head) {
        Ok(Integration::Contained) => {
            facts.push(format!("all its commits are already in {name}"));
            true
        }
        Ok(Integration::SameFiles) => {
            facts.push(format!("the files it changed already match {name}"));
            true
        }
        Ok(Integration::Differs(count)) => {
            facts.push(format!(
                "{count} {} it changed differ from {name}",
                pluralize(count, "file")
            ));
            false
        }
        Err(error) => {
            facts.push(format!("could not compare with {name} ({error})"));
            false
        }
    }
}

enum Integration {
    /// No commit of its own: everything is reachable from the base.
    Contained,
    /// Commits of its own, but every file they changed matches the base.
    SameFiles,
    /// This many changed files differ from the base.
    Differs(usize),
}

fn compare_with_base(
    git: &impl Git,
    bare: &Utf8Path,
    path: &Utf8Path,
    base: &str,
    head: &str,
) -> Result<Integration, git::Error> {
    if git.commits_ahead(bare, base, head)? == 0 {
        return Ok(Integration::Contained);
    }
    let mut differing = 0;
    for file in git.paths_changed_since_fork(path, base)? {
        if git.path_at_commit(path, head, &file)? != git.path_at_commit(path, base, &file)? {
            differing += 1;
        }
    }
    Ok(if differing == 0 {
        Integration::SameFiles
    } else {
        Integration::Differs(differing)
    })
}

fn pluralize(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        format!("{noun}s")
    }
}
