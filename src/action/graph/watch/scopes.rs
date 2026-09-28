//! Scope representation, watch sets (path -> scope classification), and event debouncing.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::graph::index::types::{is_ignored_file_path, is_ignored_path};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Base { repo: String },
    Layer { feature: String, repo: String },
}

impl Scope {
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Base { repo } => format!("base:{repo}"),
            Self::Layer { feature, repo } => format!("layer:{feature}:{repo}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirRole {
    Worktree { root: Utf8PathBuf },
    GitMeta { names: BTreeSet<String> },
}

#[derive(Debug, Default)]
pub struct WatchSet {
    dirs: BTreeMap<Utf8PathBuf, (Scope, DirRole)>,
    control_dirs: BTreeSet<Utf8PathBuf>,
}

impl WatchSet {
    /// Adds a worktree directory and its non-ignored ancestor directories for tracked files to the watch set.
    /// Returns the sorted, deduped list of directories to watch.
    pub fn add_worktree(
        &mut self,
        scope: &Scope,
        root: &Utf8Path,
        tracked: &[Utf8PathBuf],
    ) -> Vec<Utf8PathBuf> {
        let mut to_watch = BTreeSet::new();
        to_watch.insert(root.to_path_buf());

        for rel in tracked {
            let rel_std = rel.as_std_path();
            if is_ignored_path(rel_std) || is_ignored_file_path(rel_std) {
                continue;
            }
            let mut curr = rel.parent();
            while let Some(parent) = curr {
                if parent.as_str().is_empty() {
                    break;
                }
                let parent_std = parent.as_std_path();
                if is_ignored_path(parent_std) {
                    break;
                }
                to_watch.insert(root.join(parent));
                curr = parent.parent();
            }
        }

        let role = DirRole::Worktree {
            root: root.to_path_buf(),
        };
        for dir in &to_watch {
            self.dirs.insert(dir.clone(), (scope.clone(), role.clone()));
        }

        to_watch.into_iter().collect()
    }

    /// Adds a git metadata directory watching only the specified file names.
    pub fn add_git_meta(&mut self, scope: &Scope, dir: &Utf8Path, names: &[&str]) -> Utf8PathBuf {
        let name_set: BTreeSet<String> = names.iter().map(|&s| s.to_owned()).collect();
        let role = DirRole::GitMeta { names: name_set };
        let dir_buf = dir.to_path_buf();
        self.dirs.insert(dir_buf.clone(), (scope.clone(), role));
        dir_buf
    }

    /// Adds a control directory (e.g. `.ivar/features` or `.ivar/features/<f>/sessions`) to the watch set.
    pub fn add_control(&mut self, dir: &Utf8Path) -> Utf8PathBuf {
        let dir_buf = dir.to_path_buf();
        self.control_dirs.insert(dir_buf.clone());
        dir_buf
    }

    /// Checks if a path is inside or equals any watched control directory.
    #[must_use]
    pub fn is_control(&self, path: &Utf8Path) -> bool {
        self.control_dirs.iter().any(|cd| path.starts_with(cd))
    }

    /// Removes all directories associated with a scope and returns them.
    pub fn remove_scope(&mut self, scope: &Scope) -> Vec<Utf8PathBuf> {
        let mut removed = Vec::new();
        self.dirs.retain(|path, (s, _)| {
            if s == scope {
                removed.push(path.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Classifies an event path to its scope.
    #[must_use]
    pub fn classify(&self, path: &Utf8Path) -> Option<Scope> {
        let parent = path.parent()?;
        let (scope, role) = self.dirs.get(parent)?;
        match role {
            DirRole::GitMeta { names } => {
                let file_name = path.file_name()?;
                names.contains(file_name).then(|| scope.clone())
            }
            DirRole::Worktree { root } => {
                let rel = path.strip_prefix(root).ok()?.as_std_path();
                (!is_ignored_path(rel) && !is_ignored_file_path(rel)).then(|| scope.clone())
            }
        }
    }

    /// When a directory is created inside a watched worktree dir and is not ignored, registers it and returns it.
    pub fn new_dir(&mut self, path: &Utf8Path) -> Option<Utf8PathBuf> {
        let parent = path.parent()?;
        let (scope, role) = self.dirs.get(parent)?;
        match role {
            DirRole::Worktree { root } => {
                let rel = path.strip_prefix(root).ok()?.as_std_path();
                if !is_ignored_path(rel) {
                    let dir_buf = path.to_path_buf();
                    self.dirs.insert(
                        dir_buf.clone(),
                        (scope.clone(), DirRole::Worktree { root: root.clone() }),
                    );
                    Some(dir_buf)
                } else {
                    None
                }
            }
            DirRole::GitMeta { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct Debouncer {
    quiet: Duration,
    cap: Duration,
    bursts: BTreeMap<Scope, (Instant, Instant)>,
    failures: BTreeMap<Scope, u32>,
}

impl Debouncer {
    pub const QUIET: Duration = Duration::from_millis(150);
    pub const CAP: Duration = Duration::from_secs(1);
    pub const MAX_BACKOFF: Duration = Duration::from_secs(2);

    #[must_use]
    pub fn new(quiet: Duration, cap: Duration) -> Self {
        Self {
            quiet,
            cap,
            bursts: BTreeMap::new(),
            failures: BTreeMap::new(),
        }
    }

    /// Records an event for a scope. Returns `true` if this event starts a new burst.
    pub fn record(&mut self, scope: Scope, now: Instant) -> bool {
        if let Some((_, last)) = self.bursts.get_mut(&scope) {
            *last = now;
            false
        } else {
            self.bursts.insert(scope, (now, now));
            true
        }
    }

    /// Clears any accumulated failure backoff for a scope on success.
    pub fn clear_failures(&mut self, scope: &Scope) {
        self.failures.remove(scope);
    }

    /// Schedules a retry for a failed scope with exponential backoff bounded by `MAX_BACKOFF`.
    pub fn record_failure(&mut self, scope: Scope, now: Instant) {
        let attempts = self.failures.entry(scope.clone()).or_insert(0);
        *attempts = attempts.saturating_add(1);
        let backoff = (self.quiet * 2u32.saturating_pow(*attempts - 1)).min(Self::MAX_BACKOFF);
        // Schedule burst so that it becomes due after `backoff`.
        // By setting (first, last) such that now - last < quiet until now + backoff:
        let schedule_time = now + backoff - self.quiet;
        self.bursts.insert(scope, (schedule_time, schedule_time));
    }

    /// Removes and returns the scopes that are quiet for `quiet`, or whose burst is older than `cap`.
    pub fn due(&mut self, now: Instant) -> Vec<Scope> {
        let mut due = Vec::new();
        self.bursts.retain(|scope, &mut (first, last)| {
            if now >= last
                && (now.duration_since(last) >= self.quiet || now.duration_since(first) >= self.cap)
            {
                due.push(scope.clone());
                false
            } else {
                true
            }
        });
        due
    }

    /// Returns the earliest deadline among all active bursts.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.bursts
            .values()
            .map(|&(first, last)| (last + self.quiet).min(first + self.cap))
            .min()
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/watch/scopes.rs"]
mod tests;
