//! `ivar sync` — make the local hall match `ivar.json`.
//!
//! This is the verb people run every day, right after `git pull`. Everything
//! about it follows from that.
//!
//! **It is idempotent, and it proves it.** Every step compares before it acts
//! and reports [`Change::Unchanged`] when there was nothing to do. A sync that
//! rewrote files it did not need to would put spurious modifications in
//! `git status` on every run, and a tool that dirties your working tree for no
//! reason is a tool you stop running.
//!
//! **One bad repo does not stop the others.** A repo whose remote is
//! unreachable becomes a [`Change::Failed`] entry and a
//! [`Warning`](crate::error::Warning); the other seven still get set up, and
//! the process exits `1` rather than `2`. That is the warning discipline from
//! ARCHITECTURE.md, and this is the verb it was written for. A [`Failure`] here
//! is reserved for the cases where there is nothing to salvage: no hall, or a
//! hall whose `.ivar/` cannot be created.
//!
//! # What it does, in order
//!
//! 1. The hall skeleton — `.ivar/`, `.ivar/repos/`, and the `.gitignore` lines.
//! 2. Each repo in `ivar.json`: bare clone, default-branch worktree, setup
//!    script.
//! 3. Each provider: the managed block in its instruction file, its MCP
//!    config, and its official workflow commands — all materialised for a
//!    provider the hall lists and stripped for one it does not.
//!
//! Repos come before providers because the managed block names them, and a
//! block listing a repo that failed to clone would be describing a hall that
//! does not exist yet. It lists what `ivar.json` declares either way — the
//! manifest is the source of truth, and a transient network failure should not
//! rewrite the hall's documentation.
//!
//! # What it deliberately does not do
//!
//! **It does not fetch.** A hall with eight repos would make `sync` a
//! network-bound command that people learn to avoid, and there is already a
//! verb for it: `ivar repo pull`. `sync` reaches the network exactly once per
//! repo, the first time, to clone.
//!
//! **It does not remove repos dropped from `ivar.json`.** Deleting a worktree
//! can destroy uncommitted work, and a verb that runs on every `git pull` must
//! never be able to do that on its own. `ivar cleanup` (slice 8) is where
//! removal lives, and it will ask.

use std::collections::BTreeMap;
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::git::{self};
use crate::harness::{commands, config};
use crate::infra::fs;
use crate::store::gitignore;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Ctx;
use super::discover_hall;
use super::read_manifest;

mod providers;
mod repo;
mod setup;

pub(crate) use providers::{materialise_commands, sync_providers};
pub(crate) use setup::run_setup_script;

use repo::sync_repo;

/// What `ivar sync` needs.
#[derive(Debug, Clone, Default)]
pub struct SyncInput {
    /// Run every repo's setup script even when its receipt says it is current.
    /// The escape hatch for a script whose effect was undone outside `ivar` —
    /// a deleted `node_modules`, a dropped database.
    pub force_setup: bool,
}

/// What happened to one thing `sync` looked at.
///
/// Five states, and [`Self::Failed`] is the reason this is not
/// [`config::Change`]: a materialiser returns `Result` and has no failure
/// state, but a *report* has to carry one, because the whole point is that one
/// failure does not stop the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// It was not there and now is.
    Created,
    /// It was there and changed.
    Updated,
    /// It was there and was taken away.
    Removed,
    /// It was already exactly right.
    Unchanged,
    /// It could not be done. The run continued.
    Failed,
}

impl Change {
    /// The one-character marker the human surface prefixes an entry with.
    pub(crate) const fn symbol(self) -> char {
        match self {
            Self::Created => '+',
            Self::Updated => '~',
            Self::Removed => '-',
            Self::Unchanged => '=',
            Self::Failed => 'x',
        }
    }

    /// The word the counts line uses for this change.
    const fn word(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Removed => "removed",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
        }
    }

    /// Every variant, in the order the counts line prints them.
    const ALL: [Change; 5] = [
        Change::Created,
        Change::Updated,
        Change::Removed,
        Change::Unchanged,
        Change::Failed,
    ];
}

impl From<config::Change> for Change {
    fn from(change: config::Change) -> Self {
        match change {
            config::Change::Created => Self::Created,
            config::Change::Updated => Self::Updated,
            config::Change::Unchanged => Self::Unchanged,
            config::Change::Removed => Self::Removed,
        }
    }
}

impl From<commands::Change> for Change {
    fn from(change: commands::Change) -> Self {
        match change {
            commands::Change::Created => Self::Created,
            commands::Change::Updated => Self::Updated,
            commands::Change::Removed => Self::Removed,
            commands::Change::Unchanged => Self::Unchanged,
        }
    }
}

impl From<gitignore::Changed> for Change {
    fn from(changed: gitignore::Changed) -> Self {
        match changed {
            gitignore::Changed::Created => Self::Created,
            gitignore::Changed::Yes => Self::Updated,
            gitignore::Changed::No => Self::Unchanged,
        }
    }
}

/// One line of the sync report.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// What this entry is about: `hall`, `repo api`, `claude-code`. Entries are
    /// grouped under this in the human rendering, in first-seen order.
    pub surface: String,
    /// What was looked at, within the surface.
    pub label: String,
    /// What happened to it.
    pub change: Change,
    /// Anything worth saying beyond the change — a git error, why a setup
    /// script was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Entry {
    pub(crate) fn new(
        surface: impl Into<String>,
        label: impl Into<String>,
        change: Change,
    ) -> Self {
        Self {
            surface: surface.into(),
            label: label.into(),
            change,
            detail: None,
        }
    }

    pub(crate) fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// What `ivar sync` did.
///
/// `Serialize`d as-is for `--json`; [`Self::write_human`] formats this same
/// value. See ARCHITECTURE.md, "1. `action` is the unit, and it has one output
/// shape".
#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every step, in the order it happened.
    pub entries: Vec<Entry>,
}

impl SyncOutcome {
    /// How many entries ended in each [`Change`], keyed by the change itself.
    #[must_use]
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts: BTreeMap<&'static str, usize> = Change::ALL
            .iter()
            .map(|change| (change.word(), 0))
            .collect();
        for entry in &self.entries {
            *counts.entry(entry.change.word()).or_default() += 1;
        }
        counts
    }
}

impl WriteHuman for SyncOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Synced {}", self.root)?;

        // Grouped in first-seen order rather than sorted: the order things
        // happened is the order that explains them, and a repo's clone has to
        // read above the worktree that depends on it.
        let mut seen: Vec<&str> = Vec::new();
        for entry in &self.entries {
            if !seen.contains(&entry.surface.as_str()) {
                seen.push(&entry.surface);
            }
        }

        for surface in seen {
            writeln!(w)?;
            writeln!(w, "{surface}:")?;
            for entry in self.entries.iter().filter(|e| e.surface == surface) {
                match &entry.detail {
                    Some(detail) => {
                        writeln!(w, "  {} {} — {detail}", entry.change.symbol(), entry.label)?
                    }
                    None => writeln!(w, "  {} {}", entry.change.symbol(), entry.label)?,
                }
            }
        }

        let counts = self.counts();
        let rendered = Change::ALL
            .iter()
            .map(|change| {
                let word = change.word();
                format!("{word}: {}", counts.get(word).copied().unwrap_or(0))
            })
            .collect::<Vec<_>>()
            .join("  ");
        writeln!(w)?;
        writeln!(w, "{rendered}")
    }
}

/// Reconcile the hall containing [`Ctx::cwd`] against its `ivar.json`.
///
/// See the module doc comment for the order, the idempotence rule, and what
/// this deliberately does not do.
pub fn sync(ctx: &Ctx, input: SyncInput) -> Outcome<SyncOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    // The skeleton is the one part that fails the whole verb: every step below
    // writes underneath it.
    entries.push(ensure_skeleton(&layout)?);
    entries.push(Entry::new(
        "hall",
        ".gitignore",
        gitignore::ensure(&layout)?.into(),
    ));

    for repo in manifest.repos() {
        sync_repo(
            &git,
            &layout,
            repo,
            input.force_setup,
            &mut entries,
            &mut warnings,
        );
    }

    // Built once, not once per provider: the block every provider gets is the
    // same bytes, and `Provider::ALL` will only grow.
    sync_providers(&layout, &manifest, &mut entries, &mut warnings);

    Ok(Report::with_warnings(
        SyncOutcome {
            root: layout.root().to_path_buf(),
            entries,
        },
        warnings,
    ))
}

fn ensure_skeleton(layout: &Layout) -> Result<Entry, Failure> {
    let mut created = false;
    for dir in [layout.ivar_dir(), layout.repos_dir(), layout.secrets_dir()] {
        if !fs::is_dir(&dir)? {
            created = true;
        }
        fs::ensure_dir(&dir)?;
    }

    Ok(Entry::new(
        "hall",
        ".ivar/",
        if created {
            Change::Created
        } else {
            Change::Unchanged
        },
    ))
}

fn repo_names(manifest: &Manifest) -> Vec<RepoName> {
    manifest
        .repos()
        .iter()
        .map(|repo| repo.name().clone())
        .collect()
}

fn record_failure(
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
    surface: &str,
    label: &str,
    failure: Failure,
) {
    entries.push(Entry::new(surface, label, Change::Failed).detail(failure.what.clone()));
    warnings.push(Warning::new("sync.step_failed", surface, failure.what));
}

#[cfg(test)]
#[path = "../../../tests/unit/action/sync.rs"]
mod tests;
