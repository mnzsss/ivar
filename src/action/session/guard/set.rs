//! The writable set: which paths a session may write, and the hall root's protected paths.

use crate::domain::feature::Feature;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// The set of paths a session is allowed to write into: its view dir, its
/// feature directory (for feature sessions), the worktrees of promoted repos,
/// the hall sources (`.ivar/skills`, `.ivar/skills-local`, `.ivar/setups`), and
/// the hall root outside `.ivar/` except its protected paths.
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    pub(super) view_dir: Utf8PathBuf,
    pub(super) feature_dir: Option<Utf8PathBuf>,
    pub(super) sessions_dir: Option<Utf8PathBuf>,
    pub(super) worktrees: Vec<Utf8PathBuf>,
    pub(super) descendant_feature_dirs: Vec<Utf8PathBuf>,
    pub(super) descendant_sessions_dirs: Vec<Utf8PathBuf>,
    pub(super) descendant_worktrees: Vec<Utf8PathBuf>,
    pub(super) hall_sources: Vec<Utf8PathBuf>,
    pub(super) hall: HallRoot,
}

/// The hall root minus `.ivar/` and the protected git and hook-config paths:
/// shared hall files every session may write.
#[derive(Debug, Clone)]
pub(super) struct HallRoot {
    root: Utf8PathBuf,
    ivar_dir: Utf8PathBuf,
    protected: Vec<Utf8PathBuf>,
}

impl std::fmt::Display for HallRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (except {}", self.root, self.ivar_dir)?;
        for protected in &self.protected {
            write!(f, ", {protected}")?;
        }
        f.write_str(")")
    }
}

impl HallRoot {
    pub(super) fn new(layout: &Layout) -> Self {
        Self {
            root: canonicalize_lenient(layout.root()),
            ivar_dir: canonicalize_lenient(&layout.ivar_dir()),
            protected: layout
                .guard_protected_paths()
                .iter()
                .map(|path| canonicalize_lenient(path))
                .collect(),
        }
    }

    pub(super) fn allows(&self, canonical: &Utf8Path) -> bool {
        within(canonical, &self.root)
            && !canonical.starts_with(&self.ivar_dir)
            && !self
                .protected
                .iter()
                .any(|protected| canonical.starts_with(protected))
    }

    fn holds_protected(&self, canonical: &Utf8Path) -> bool {
        self.protected
            .iter()
            .any(|protected| protected.starts_with(canonical))
    }

    // ponytail: Landlock grants whole subtrees and cannot exclude a sub-path,
    // so the root is expanded into the entries present at launch, descending
    // into each real dir that holds a protected path. A new entry directly in
    // an expanded dir (the hall root, `.git`, `.claude`) is kernel-denied
    // until relaunch; for `.git` that includes `index.lock`, so a sandboxed
    // `git commit` in the hall fails.
    fn entries(&self) -> Result<Vec<Utf8PathBuf>, Failure> {
        let mut granted = Vec::new();
        self.expand(&self.root, &mut granted)?;
        Ok(granted)
    }

    fn expand(&self, dir: &Utf8Path, granted: &mut Vec<Utf8PathBuf>) -> Result<(), Failure> {
        let entries = crate::infra::fs::read_dir(dir).map_err(|source| {
            Failure::failed(
                "guard.unreadable_hall_root",
                format!("could not read hall dir `{dir}`: {source}"),
            )
        })?;
        for entry in entries {
            let canonical = canonicalize_lenient(&entry);
            if canonical == self.root || !self.allows(&canonical) {
                continue;
            }
            if !self.holds_protected(&canonical) {
                granted.push(canonical);
            } else if entry.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
                self.expand(&entry, granted)?;
            }
        }
        Ok(())
    }
}

/// Leniently canonicalise `path`. If canonicalisation fails (e.g. for a
/// file that does not exist yet), walk to the nearest existing ancestor,
/// canonicalise it, and append the remaining non-existent components,
/// falling back to the raw path if no ancestor canonicalises. A result that
/// still holds a `..` is unknowable, so it becomes the empty path, which no
/// writable root contains.
///
/// A dangling symlink on the way resolves to its target, so a write through
/// it is judged by where the bytes would land.
pub(crate) fn canonicalize_lenient(path: &Utf8Path) -> Utf8PathBuf {
    let resolved = canonicalize_within_hops(path, MAX_SYMLINK_HOPS);
    if resolved
        .components()
        .any(|component| matches!(component, camino::Utf8Component::ParentDir))
    {
        // A `..` that survived resolution sits under a missing directory, so
        // where the bytes land cannot be known yet: the empty path lies under
        // no writable root.
        return Utf8PathBuf::new();
    }
    resolved
}

// The empty path is a prefix of every path, so a root that resolved to it
// must grant nothing rather than everything.
pub(super) fn within(path: &Utf8Path, root: &Utf8Path) -> bool {
    !root.as_str().is_empty() && path.starts_with(root)
}

pub(super) const MAX_SYMLINK_HOPS: usize = 40;

pub(super) fn canonicalize_within_hops(path: &Utf8Path, hops_left: usize) -> Utf8PathBuf {
    let mut existing = path;
    let mut tail = Vec::new();
    while !existing.exists() {
        if existing.is_symlink() {
            let (Some(hops_left), Ok(target)) =
                (hops_left.checked_sub(1), existing.read_link_utf8())
            else {
                // A link loop or an unreadable link: the empty path lies under
                // no writable root, so every set denies it.
                return Utf8PathBuf::new();
            };
            let target = match existing.parent() {
                Some(dir) => dir.join(target),
                None => target,
            };
            let mut resolved = canonicalize_within_hops(&target, hops_left);
            for name in tail.into_iter().rev() {
                resolved.push(name);
            }
            return resolved;
        }
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name);
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    if let Ok(mut canonical) = existing.canonicalize_utf8() {
        for name in tail.into_iter().rev() {
            canonical.push(name);
        }
        return canonical;
    }
    path.to_path_buf()
}

pub(super) fn hall_sources(layout: &Layout) -> Vec<Utf8PathBuf> {
    [
        layout.hall_skills(),
        layout.hall_skills_local(),
        layout.hall_setups(),
    ]
    .into_iter()
    .map(|path| canonicalize_lenient(&path))
    .collect()
}

impl WritableSet {
    /// Build the writable set from the session's view dir, the feature's
    /// directory, and the feature's promoted repos. The view dir, feature dir,
    /// and each worktree are canonicalised to prevent symlink escapes.
    pub(crate) fn from_session(
        layout: &Layout,
        feature: &Feature,
        view_dir: &Utf8Path,
    ) -> Result<Self, Failure> {
        let view_dir = view_dir.canonicalize_utf8().map_err(|source| {
            Failure::failed(
                "guard.unresolvable_view_dir",
                format!("could not canonicalise view dir `{view_dir}`: {source}"),
            )
        })?;
        let feat_dir_raw = layout.feature_dir(&feature.name);
        let feature_dir = canonicalize_lenient(&feat_dir_raw);
        let sessions_dir = canonicalize_lenient(&layout.feature_sessions_dir(&feature.name));
        let worktrees = feature
            .promotions
            .keys()
            .map(|repo| {
                let wt = layout.repo_worktree(repo, &feature.branch);
                wt.canonicalize_utf8().map_err(|source| {
                    Failure::failed(
                        "guard.unresolvable_worktree",
                        format!("could not canonicalise worktree `{wt}`: {source}"),
                    )
                })
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        let mut descendant_feature_dirs = Vec::new();
        let mut descendant_sessions_dirs = Vec::new();
        let mut descendant_worktrees = Vec::new();

        if let Ok(descendants) = crate::action::feature::descendants(layout, &feature.name) {
            for desc in descendants {
                let desc_feat_dir = canonicalize_lenient(&layout.feature_dir(&desc.name));
                let desc_sess_dir = canonicalize_lenient(&layout.feature_sessions_dir(&desc.name));
                descendant_feature_dirs.push(desc_feat_dir);
                descendant_sessions_dirs.push(desc_sess_dir);
                for repo in desc.promotions.keys() {
                    let wt = layout.repo_worktree(repo, &desc.branch);
                    descendant_worktrees.push(canonicalize_lenient(&wt));
                }
            }
        }

        Ok(Self {
            view_dir,
            feature_dir: Some(feature_dir),
            sessions_dir: Some(sessions_dir),
            worktrees,
            descendant_feature_dirs,
            descendant_sessions_dirs,
            descendant_worktrees,
            hall_sources: hall_sources(layout),
            hall: HallRoot::new(layout),
        })
    }

    /// Build the writable set for a discovery session: the view dir, the
    /// canonical hall sources, and the hall root outside `.ivar/`.
    pub(crate) fn from_discovery(layout: &Layout, view_dir: &Utf8Path) -> Result<Self, Failure> {
        let view_dir = view_dir.canonicalize_utf8().map_err(|source| {
            Failure::failed(
                "guard.unresolvable_view_dir",
                format!("could not canonicalise view dir `{view_dir}`: {source}"),
            )
        })?;
        Ok(Self {
            view_dir,
            feature_dir: None,
            sessions_dir: None,
            worktrees: Vec::new(),
            descendant_feature_dirs: Vec::new(),
            descendant_sessions_dirs: Vec::new(),
            descendant_worktrees: Vec::new(),
            hall_sources: hall_sources(layout),
            hall: HallRoot::new(layout),
        })
    }

    /// Whether `path` is inside the view dir, the hall sources (`.ivar/skills`,
    /// `.ivar/skills-local`, `.ivar/setups`), the feature directory when
    /// applicable, one of the promoted worktrees, or the hall root outside
    /// `.ivar/` and its protected paths.
    /// The input path is canonicalised (with parent fallback for not-yet-existing
    /// files) so symlinks cannot escape the set on platforms like macOS where
    /// `/tmp` or `/var` are symlinks.
    pub(crate) fn allows(&self, path: &Utf8Path) -> bool {
        let canonical = canonicalize_lenient(path);
        if within(&canonical, &self.view_dir) {
            return true;
        }
        if let Some(sessions_dir) = &self.sessions_dir
            && canonical.starts_with(sessions_dir)
        {
            return false;
        }
        if self
            .descendant_sessions_dirs
            .iter()
            .any(|sd| canonical.starts_with(sd))
        {
            return false;
        }
        if self
            .feature_dir
            .as_ref()
            .is_some_and(|fd| within(&canonical, fd))
        {
            return true;
        }
        if self
            .descendant_feature_dirs
            .iter()
            .any(|fd| within(&canonical, fd))
        {
            return true;
        }
        if self.worktrees.iter().any(|wt| within(&canonical, wt)) {
            return true;
        }
        if self
            .descendant_worktrees
            .iter()
            .any(|wt| within(&canonical, wt))
        {
            return true;
        }
        if self
            .hall_sources
            .iter()
            .any(|root| within(&canonical, root))
        {
            return true;
        }
        self.hall.allows(&canonical)
    }

    /// The view dir — the canonical root of this set.
    pub(crate) fn view_dir(&self) -> &Utf8Path {
        &self.view_dir
    }

    /// The session's scratch dir — where an agent's temporary files belong.
    ///
    /// Derived, never stored: `Layout::session_scratch` is the single owner
    /// of the path, and this set already holds the canonical view dir.
    pub(crate) fn scratch_dir(&self) -> Utf8PathBuf {
        Layout::session_scratch(&self.view_dir)
    }

    /// Return the write-allowed root paths: view dir, canonical hall sources,
    /// feature dir (if present), every promoted repo worktree, and each
    /// hall-root entry outside `.ivar/`. Note that `sessions_dir` is an
    /// exclusion boundary under `feature_dir` and is not a root.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if a hall dir the expansion walks cannot be read:
    /// granting less than the guard allows would fail writes silently.
    pub(crate) fn roots(&self) -> Result<Vec<Utf8PathBuf>, Failure> {
        Ok(std::iter::once(self.view_dir.clone())
            .chain(self.feature_dir.clone())
            .chain(self.descendant_feature_dirs.iter().cloned())
            .chain(self.worktrees.iter().cloned())
            .chain(self.descendant_worktrees.iter().cloned())
            .chain(self.hall_sources.iter().cloned())
            .chain(self.hall.entries()?)
            .collect())
    }

    /// Build a `WritableSet` from explicit parts. Test-only.
    #[cfg(test)]
    pub(crate) fn from_parts(
        view_dir: Utf8PathBuf,
        feature_dir: Option<&Utf8Path>,
        worktrees: &[Utf8PathBuf],
    ) -> Self {
        let view_dir = canonicalize_lenient(&view_dir);
        let sessions_dir = feature_dir.map(|fd| canonicalize_lenient(&fd.join("sessions")));
        let feature_dir = feature_dir.map(canonicalize_lenient);
        let worktrees = worktrees.iter().map(|w| canonicalize_lenient(w)).collect();
        Self {
            feature_dir,
            sessions_dir,
            worktrees,
            descendant_feature_dirs: Vec::new(),
            descendant_sessions_dirs: Vec::new(),
            descendant_worktrees: Vec::new(),
            hall_sources: Vec::new(),
            // A hall whose `.ivar` is its root allows nothing and grants nothing.
            hall: HallRoot {
                root: view_dir.clone(),
                ivar_dir: view_dir.clone(),
                protected: Vec::new(),
            },
            view_dir,
        }
    }
}
