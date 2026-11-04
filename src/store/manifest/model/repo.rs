use serde::{Deserialize, Serialize};

use crate::domain::name::{BranchName, RepoName};

/// One repo a hall knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    /// A unique, non-empty, single-segment identifier for this repo.
    #[schemars(example = &"my-repo")]
    pub(super) name: RepoName,
    /// The git remote URL to clone from. Must not be empty.
    #[schemars(example = &"https://github.com/org/repo.git")]
    pub(super) url: String,
    /// The branch a fresh worktree defaults to (e.g. "main").
    #[schemars(example = &"main")]
    pub(super) default_branch: BranchName,
    /// The repo's ordered verification checks, run via `bash -lc` in the
    /// relevant worktree when this repo is integrated or delivered. Empty
    /// means "no checks" — the v1 common case, and deliberately omitted from
    /// the on-disk shape. `#[serde(default)]` so a v1 repo still deserialises.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) checks: Vec<String>,
    /// Optional explicit prefix that remaps the local git branches under `refs/heads/`.
    /// Must match `repos/<name>/` when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ref_prefix: Option<String>,
}

impl Repo {
    /// Build a `Repo`. `url` stays a plain `String` rather than one of
    /// `domain::name`'s newtypes: it is a git remote, never joined onto a path,
    /// so none of the path-safety rules apply to it.
    ///
    /// It must still be non-empty, and that is checked by
    /// [`Manifest::validate`](super::Manifest::validate) alongside the other invariants rather than here —
    /// so it is enforced on **read** too, which is the case that matters, since
    /// this file is hand-edited. An empty `url` is the difference between a
    /// `Failure` naming the offending repo and a bare `git clone` error the
    /// first time someone runs `ivar sync`.
    ///
    /// The v1 call shape is preserved: a repo built through this constructor
    /// carries no checks.
    #[must_use]
    pub fn new(name: RepoName, url: impl Into<String>, default_branch: BranchName) -> Self {
        Self {
            name,
            url: url.into(),
            default_branch,
            checks: Vec::new(),
            ref_prefix: None,
        }
    }

    /// This repo's name. Unique among a manifest's repos — see
    /// [`Manifest::validate`](super::Manifest::validate).
    #[must_use]
    pub fn name(&self) -> &RepoName {
        &self.name
    }

    /// The git remote URL this repo clones from.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The branch a fresh worktree of this repo defaults to.
    #[must_use]
    pub fn default_branch(&self) -> &BranchName {
        &self.default_branch
    }

    /// The repo's ordered verification checks, in execution order. Empty when
    /// the hall declared none for this repo.
    #[must_use]
    pub fn checks(&self) -> &[String] {
        &self.checks
    }

    /// Return this repo with `checks` as its verification commands, replacing
    /// whatever it carried before. The original is untouched.
    ///
    /// `cfg(test)`: a fixture builder — the CLI parses checks out of
    /// `ivar.json`, it never grafts them onto a `Repo` in memory.
    #[cfg(test)]
    #[must_use]
    pub fn with_checks(mut self, checks: Vec<String>) -> Self {
        self.checks = checks;
        self
    }

    #[must_use]
    pub fn ref_prefix(&self) -> Option<&str> {
        self.ref_prefix.as_deref()
    }

    #[must_use]
    pub fn with_ref_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.ref_prefix = Some(prefix.into());
        self
    }
}

#[must_use]
pub fn local_ref_prefix(name: &RepoName) -> String {
    format!("repos/{name}/")
}
