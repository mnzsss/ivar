use std::collections::BTreeMap;
use std::fmt;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// What is at one path, as far as the receipt records it.
///
/// Three states, not a `bool`: a symlink whose target changed is a real edit
/// that a file-content hash would miss entirely, and "absent" has to be a
/// value rather than a missing map entry so a *removal* can be evidence
/// rather than a gap.
///
/// Directories are never a state here — untracked directories are expanded to
/// their files before evidence is recorded, so every path in a receipt names
/// one blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathState {
    /// Nothing is at the path.
    Absent,
    /// A regular file.
    File,
    /// A symbolic link.
    Symlink,
}

/// One path's recorded state: what it is, its git filemode, and the hash of
/// its content.
///
/// **No source bytes, ever.** A hash proves a change without turning the
/// receipt into an archive of someone's working tree, which is the whole of
/// N-PRIVACY.
///
/// For a [`PathState::Symlink`] the hash is over the *link target's* bytes —
/// the same thing git stores in a symlink blob — so a state read from the
/// worktree and one read from a commit compare equal when they should.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEvidence {
    /// What is at the path.
    pub state: PathState,
    /// The git filemode (`100644`, `100755`, `120000`), when the path exists.
    /// Recorded because flipping the executable bit is a change no content
    /// hash can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// SHA-256 of the content, when the path exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl PathEvidence {
    /// Nothing is at the path.
    #[must_use]
    pub const fn absent() -> Self {
        Self {
            state: PathState::Absent,
            mode: None,
            hash: None,
        }
    }

    /// A regular file with `mode` and content hash `hash`.
    #[must_use]
    pub fn file(mode: u32, hash: impl Into<String>) -> Self {
        Self {
            state: PathState::File,
            mode: Some(mode),
            hash: Some(hash.into()),
        }
    }

    /// A symlink whose target's bytes hash to `hash`.
    #[must_use]
    pub fn symlink(hash: impl Into<String>) -> Self {
        Self {
            state: PathState::Symlink,
            mode: Some(0o120_000),
            hash: Some(hash.into()),
        }
    }

    /// Whether anything is at the path.
    #[must_use]
    pub const fn exists(&self) -> bool {
        !matches!(self.state, PathState::Absent)
    }
}

/// How one path changed between a run's baseline and a finish checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Nothing was there at start; something is there now.
    Added,
    /// Something was there at both boundaries, and its content, mode or kind
    /// differs.
    Modified,
    /// Something was there at start; nothing is there now.
    Removed,
    /// The path diverged from the starting commit at start, and now matches
    /// that commit again — inherited dirty work the run undid. Called out
    /// separately because it is neither "modified into something new" nor
    /// harmless: it destroyed work the run did not create.
    Reverted,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Removed => "removed",
            Self::Reverted => "reverted",
        };
        f.pad(name)
    }
}

/// Classify one path from the three states that describe it.
///
/// `start` is what the worktree held when the run began, `commit` is what the
/// run's starting commit holds for that path, and `final_state` is the
/// worktree now. `None` means the run did not change this path and it must
/// not appear in the diff — which is exactly how inherited dirty work that
/// nobody touched avoids being blamed on the run.
///
/// The `Reverted` test comes first and is deliberately narrow: the path must
/// have *diverged* from the starting commit at start (`start != commit`) and
/// match it now. A run that merely edits a clean file back and forth is
/// `None`, not `Reverted`.
#[must_use]
pub fn classify_change(
    start: &PathEvidence,
    commit: &PathEvidence,
    final_state: &PathEvidence,
) -> Option<ChangeKind> {
    if start == final_state {
        return None;
    }
    if start != commit && final_state == commit {
        return Some(ChangeKind::Reverted);
    }
    match (start.exists(), final_state.exists()) {
        (false, true) => Some(ChangeKind::Added),
        (true, false) => Some(ChangeKind::Removed),
        _ => Some(ChangeKind::Modified),
    }
}

/// One repo's state when a run started: the commit it was on, plus every path
/// that already diverged from that commit.
///
/// Clean tracked paths are deliberately absent. Their baseline content is
/// addressable from `head` for as long as the commit exists, so copying it
/// here would be storage for nothing — and the paths that *are* here are
/// exactly the ones no commit can describe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoBaseline {
    /// The worktree this baseline was read from.
    pub worktree: Utf8PathBuf,
    /// The commit `HEAD` named at start.
    pub head: String,
    /// Every path dirty or untracked at start, worktree-relative, with the
    /// state it was in. Ordered by path, so two reads render identically.
    #[serde(default)]
    pub dirty: BTreeMap<Utf8PathBuf, PathEvidence>,
}

/// Every promoted repo's state when the run started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBaseline {
    /// Repo name → that repo's baseline, ordered by name.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoBaseline>,
}

impl RunBaseline {
    /// A baseline over no repos — what a legacy import gets, since the
    /// evidence it would need was never recorded.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// One path's entry in a run diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathChange {
    /// How the path changed.
    pub kind: ChangeKind,
    /// What is at the path now.
    pub final_state: PathEvidence,
}

/// What one repo looks like at a finish checkpoint, relative to its baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoDiff {
    /// The commit `HEAD` names now. Differs from the baseline's `head` after
    /// a commit, amend, rebase, reset, or branch switch — all of which the
    /// path set below already accounts for.
    pub head: String,
    /// Every changed path, ordered by path. One map rather than four sets:
    /// a path has exactly one classification, and four sets would let it hold
    /// two.
    #[serde(default)]
    pub changes: BTreeMap<Utf8PathBuf, PathChange>,
}

/// What every repo looks like at a finish checkpoint, relative to the run's
/// baseline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDiff {
    /// Repo name → that repo's diff, ordered by name.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoDiff>,
}

impl RunDiff {
    /// Whether any repo changed at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repos.values().all(|repo| repo.changes.is_empty())
    }
}
