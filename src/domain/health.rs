//! Hall health — the pure derivation that `ivar status` renders.
//!
//! What "healthy" means, computed from plain facts (does the manifest exist?
//! are the repos cloned? do the worktrees match?), with no I/O — the caller
//! gathers the facts, this module turns them into a verdict.
//!
//! The ladder is deliberately coarse: four states, and anything below
//! [`Health::Operational`] carries a human-readable sentence naming the
//! first thing that stands between the hall and usable.

/// The overall verdict for a hall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// No `ivar.json` — the directory is not a hall yet. `ivar init` is the
    /// way in.
    Uninitialized,
    /// Everything the manifest declares exists on disk. A hall an agent can
    /// work in.
    Operational,
    /// The manifest exists but the local copy is behind — at least one repo
    /// has a remote ref the bare clone does not. `ivar repo pull` catches up.
    Stale,
    /// The manifest exists but something it declares is missing or broken —
    /// a repo never cloned, a worktree gone, a corrupt record.
    Degraded,
}

impl Health {
    /// Derive the verdict from the observed facts.
    ///
    /// - no manifest → [`Self::Uninitialized`].
    /// - else, every declared repo cloned and every default worktree
    ///   present → [`Self::Operational`].
    /// - every repo cloned but at least one is behind its remote ref →
    ///   [`Self::Stale`].
    /// - at least one repo not cloned, or a declared worktree missing →
    ///   [`Self::Degraded`].
    ///
    /// `repos` is one entry per repo in the manifest, already observed.
    #[must_use]
    pub fn derive(repos: &[RepoHealth]) -> Self {
        if repos.is_empty() {
            // An empty manifest is a valid (if empty) hall — nothing to be
            // stale or degraded about.
            return Self::Operational;
        }

        let any_missing = repos.iter().any(|repo| !repo.bare_cloned);
        if any_missing {
            return Self::Degraded;
        }

        let any_worktree_missing = repos
            .iter()
            .any(|repo| repo.default_worktree_present == Some(false));
        if any_worktree_missing {
            return Self::Degraded;
        }

        let any_stale = repos.iter().any(|repo| repo.ahead_of_bare);
        if any_stale {
            return Self::Stale;
        }

        Self::Operational
    }
}

/// The observed state of one repo, as far as health is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoHealth {
    /// Whether the bare clone exists.
    pub bare_cloned: bool,
    /// Whether the default worktree exists. `None` when the repo is not
    /// cloned (the question has no answer).
    pub default_worktree_present: Option<bool>,
    /// Whether the remote has refs the bare clone lacks — i.e. a `pull`
    /// would change something.
    pub ahead_of_bare: bool,
}

#[cfg(test)]
#[path = "../../tests/unit/domain/health.rs"]
mod tests;
