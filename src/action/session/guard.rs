//! The session write guard: determines which files a session may write.
//!
use crate::domain::feature::Feature;
use crate::domain::graph::{MissEvent, MissKind, UsageEvent, UsageSource};
pub use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// The set of paths a session is allowed to write into: its view dir, its
/// feature directory (for feature sessions), the worktrees of promoted repos,
/// the hall sources (`.ivar/skills`, `.ivar/skills-local`, `.ivar/setups`), and
/// the hall root outside `.ivar/` except its protected paths.
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    view_dir: Utf8PathBuf,
    feature_dir: Option<Utf8PathBuf>,
    sessions_dir: Option<Utf8PathBuf>,
    worktrees: Vec<Utf8PathBuf>,
    hall_sources: Vec<Utf8PathBuf>,
    hall: HallRoot,
}

/// The hall root minus `.ivar/` and the protected git and hook-config paths:
/// shared hall files every session may write.
#[derive(Debug, Clone)]
struct HallRoot {
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
    fn new(layout: &Layout) -> Self {
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

    fn allows(&self, canonical: &Utf8Path) -> bool {
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
fn within(path: &Utf8Path, root: &Utf8Path) -> bool {
    !root.as_str().is_empty() && path.starts_with(root)
}

const MAX_SYMLINK_HOPS: usize = 40;

fn canonicalize_within_hops(path: &Utf8Path, hops_left: usize) -> Utf8PathBuf {
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

fn hall_sources(layout: &Layout) -> Vec<Utf8PathBuf> {
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
        Ok(Self {
            view_dir,
            feature_dir: Some(feature_dir),
            sessions_dir: Some(sessions_dir),
            worktrees,
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
            .feature_dir
            .as_ref()
            .is_some_and(|fd| within(&canonical, fd))
        {
            return true;
        }
        if self.worktrees.iter().any(|wt| within(&canonical, wt)) {
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
            .chain(self.worktrees.iter().cloned())
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

/// Whether `tool` is a structured write — a tool whose whole purpose is to put
/// bytes on disk at a path it names.
///
/// Matched on a normalised name so `NotebookEdit`, `notebook_edit` and
/// `notebook-edit` are one tool rather than three spellings, one of which is
/// always the one a provider actually sends. The list is explicit and
/// closed: a tool that writes and is not named here is a gap, and the test
/// beside this function is where that gap is closed.
fn is_structured_write(tool: &str) -> bool {
    let normalised: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(
        normalised.as_str(),
        "write" | "edit" | "multiedit" | "notebookedit" | "applypatch" | "patch"
    )
}

/// Check whether a path string starts with an RFC 3986 URI scheme (`<scheme>://`).
/// Schemes match `^[a-zA-Z][a-zA-Z0-9+.-]*://`.
fn has_uri_scheme(s: &str) -> bool {
    let Some((scheme, _rest)) = s.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        }
        _ => false,
    }
}

/// What the guard managed to resolve for one tool request.
///
/// Borrows its set: a caller may decide twice about the same session, and
/// moving the set into the resolution would forbid that.
///
/// `Unresolved` carries the live sessions' scratch dirs for the *message*
/// alone. An unresolved structured write is always denied, so these paths
/// never widen what may be written (N-NO-WIDEN).
pub(crate) enum Resolution<'a> {
    Resolved(&'a WritableSet),
    Unresolved { scratch_dirs: Vec<Utf8PathBuf> },
    Ambiguous { features: Vec<String> },
}

/// Decide whether a tool request is allowed inside the session.
///
/// Structured write tools are checked against the writable set; everything
/// else is allowed. Shell is not classified here — it is a separate layer.
///
/// An absent set means neither the cwd nor the target resolved a session, so
/// the denial names both: the caller's next move is to check where the target
/// lives, not only where the agent stands.
pub(crate) fn decide(
    resolution: &Resolution<'_>,
    req: &ToolRequest,
    target: Option<&Utf8Path>,
) -> GuardDecision {
    if !is_structured_write(&req.tool) {
        return GuardDecision::Allow;
    }
    if req
        .file_path
        .as_ref()
        .is_some_and(|p| has_uri_scheme(p.as_str()))
    {
        return GuardDecision::Allow;
    }
    match resolution {
        Resolution::Resolved(set) => {
            if target.is_some_and(|path| set.allows(path)) {
                GuardDecision::Allow
            } else {
                let guidance = denial_guidance(Some(set), target, req.file_path.as_deref());
                GuardDecision::Deny {
                    reason: format!(
                        "writable set: {}; {guidance}",
                        std::iter::once(set.view_dir().to_string())
                            .chain(set.feature_dir.as_ref().map(|f| f.to_string()))
                            .chain(set.worktrees.iter().map(|w| w.to_string()))
                            .chain(set.hall_sources.iter().map(|h| h.to_string()))
                            .chain([set.hall.to_string()])
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                }
            }
        }
        Resolution::Unresolved { scratch_dirs } => GuardDecision::Deny {
            reason: unresolved_reason(scratch_dirs),
        },
        Resolution::Ambiguous { features } => GuardDecision::Deny {
            reason: format!(
                "write target matches session-specific roots of multiple conflicting features: {}",
                features.join(", ")
            ),
        },
    }
}

/// Classify a write denial to tell the agent where the write can go instead.
fn denial_guidance(
    set: Option<&WritableSet>,
    target: Option<&Utf8Path>,
    original: Option<&Utf8Path>,
) -> String {
    // Check target path or original path for Claude scratchpad
    let is_scratchpad = |p: &Utf8Path| -> bool {
        let s = p.as_str();
        (s.contains("/claude-") || s.contains("\\claude-")) && s.contains("/scratchpad")
            || s.contains("scratchpad") && s.contains("/tmp/")
    };

    if target.is_some_and(is_scratchpad) || original.is_some_and(is_scratchpad) {
        if let Some(set) = set {
            return format!(
                "scratchpad writes are not permitted; temporary files belong in {}",
                set.scratch_dir()
            );
        }
        return "scratchpad writes are not permitted; temporary files belong in the session's scratch directory".to_owned();
    }

    // Check for Claude Code auto-memory (~/.claude/projects/.../memory/...)
    let is_auto_memory = |p: &Utf8Path| -> bool {
        let s = p.as_str();
        s.contains(".claude/projects/") && s.contains("/memory")
    };

    if target.is_some_and(is_auto_memory) || original.is_some_and(is_auto_memory) {
        return "auto-memory writes outside the hall are not permitted; durable notes belong in hall docs or .ivar/skills".to_owned();
    }

    let scratch_suffix = match set {
        Some(set) => format!("; temporary files belong in {}", set.scratch_dir()),
        None => "; temporary files belong in the session's scratch directory".to_owned(),
    };

    if let Some(target) = target
        && let Some(msg) = classify_layout_path_denial(target, set, &scratch_suffix)
    {
        return msg;
    }

    if let Some(set) = set {
        format!("temporary files belong in {}", set.scratch_dir())
    } else {
        "temporary files belong in the session's scratch directory".to_owned()
    }
}

fn classify_layout_path_denial(
    target: &Utf8Path,
    set: Option<&WritableSet>,
    scratch_suffix: &str,
) -> Option<String> {
    let mut current = target;
    let existing_ancestor = loop {
        if current.exists() {
            break current;
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break current,
        }
    };

    let layout = Layout::discover(existing_ancestor).ok()??;

    // Check protected hall paths
    let protected_paths: Vec<Utf8PathBuf> = layout
        .guard_protected_paths()
        .into_iter()
        .map(|p| canonicalize_lenient(&p))
        .collect();

    if protected_paths
        .iter()
        .any(|p| target == p || target.starts_with(p))
    {
        return Some(format!(
            "this path is ivar-managed and protected; change it through the owning `ivar` command{scratch_suffix}"
        ));
    }

    // Check if target is in an unpromoted repo
    let repos_dir = canonicalize_lenient(&layout.repos_dir());
    if target.starts_with(&repos_dir) {
        if let Ok(entries) = crate::infra::fs::read_dir(&layout.repos_dir()) {
            for entry in entries {
                let canonical_entry = canonicalize_lenient(&entry);
                if target.starts_with(&canonical_entry)
                    && let Some(repo_name) = entry.file_name()
                {
                    return Some(format!(
                        "repo `{repo_name}` is not promoted in this feature; run `ivar feature promote {repo_name}` to make it writable{scratch_suffix}"
                    ));
                }
            }
        }
        return Some(format!(
            "this repo is not promoted in this feature; run `ivar feature promote <repo>` to make it writable{scratch_suffix}"
        ));
    }

    // Check if target is inside another session view dir or feature dir
    let features_dir = canonicalize_lenient(&layout.features_dir());
    let discovery_sessions_dir = canonicalize_lenient(&layout.discovery_sessions_dir());

    if target.starts_with(&discovery_sessions_dir) || target.starts_with(&features_dir) {
        if let Some(set) = set {
            if target.starts_with(&features_dir)
                && let Some(fd) = &set.feature_dir
                && !target.starts_with(fd)
            {
                return Some(format!(
                    "writes to another feature's directory are not permitted; writes belong in your feature directory `{fd}` or view dir `{}`{scratch_suffix}",
                    set.view_dir()
                ));
            }
            return Some(format!(
                "writes to another session's view dir are not permitted; writes belong in your view dir `{}`{scratch_suffix}",
                set.view_dir()
            ));
        }
        return Some(format!(
            "writes to another session's view dir or feature are not permitted; writes belong in your own view dir or feature directory{scratch_suffix}"
        ));
    }

    // Check other .ivar/ state
    let ivar_dir = canonicalize_lenient(&layout.ivar_dir());
    if target.starts_with(&ivar_dir) {
        return Some(format!(
            "`.ivar/` state is managed by ivar; use the matching `ivar` command{scratch_suffix}"
        ));
    }

    None
}

/// The unresolved denial's reason. The first sentence is unchanged and
/// load-bearing: five test assertions and two documents quote it.
fn unresolved_reason(scratch_dirs: &[Utf8PathBuf]) -> String {
    const SENTENCE: &str = "no ivar session resolves from the cwd or the target path";
    match scratch_dirs {
        [] => format!("{SENTENCE}; this hall has no live session"),
        [only] => format!("{SENTENCE}; temporary files belong in {only}"),
        many => format!(
            "{SENTENCE}; temporary files belong in a live session's scratch dir: {}",
            many.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn relative_no_session_reason(relative_path: &Utf8Path, cwd: Option<&Utf8Path>) -> String {
    match cwd {
        Some(cwd) => format!(
            "relative path `{relative_path}` was resolved against `{cwd}`, which belongs to no ivar session; use an absolute path inside your session's worktree or view dir"
        ),
        None => format!(
            "relative path `{relative_path}` was resolved without a cwd, which belongs to no ivar session; use an absolute path inside your session's worktree or view dir"
        ),
    }
}

/// Every live session's scratch dir, for the unresolved message.
///
/// Called only after both resolution attempts have failed, so an allowed
/// write never pays for this walk (N-ALLOW-PATH-COST).
fn live_scratch_dirs(from: Option<&Utf8Path>) -> Vec<Utf8PathBuf> {
    let Some(from) = from else {
        return Vec::new();
    };
    let Ok(Some(layout)) = Layout::discover(from) else {
        return Vec::new();
    };
    let Ok(sessions) = super::lookup::list_all(&layout) else {
        return Vec::new();
    };
    sessions
        .into_iter()
        .filter(|session| session.state.is_some())
        .map(|session| Layout::session_scratch(&session.view_dir))
        .collect()
}

/// Resolve the target path for a tool request: absolute paths are leniently
/// canonicalized, relative paths are joined to payload cwd and leniently
/// canonicalized, and absent cwd/target returns `None`. Targets with an RFC 3986
/// URI scheme (e.g. `xd://...`, `memory://...`) return `None` so they are not
/// treated as filesystem targets.
fn resolve_target(cwd: Option<&Utf8Path>, file_path: &Utf8Path) -> Option<Utf8PathBuf> {
    if has_uri_scheme(file_path.as_str()) {
        return None;
    }
    let absolute = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        cwd?.join(file_path)
    };
    Some(canonicalize_lenient(&absolute))
}

/// Target resolution outcome when resolving a writable set from a target path.
#[derive(Debug)]
enum TargetResolution {
    None,
    SharedHall(WritableSet),
    Unique(WritableSet),
    Ambiguous(Vec<String>),
}

/// Resolve the authoritative writable set for a target path when cwd resolves
/// no session.
fn match_feature_sessions(
    layout: &Layout,
    target: &Utf8Path,
) -> Vec<(
    crate::domain::name::FeatureName,
    crate::domain::session::SessionRef,
)> {
    let mut feature_matching_sessions = Vec::new();
    let Ok(entries) = crate::infra::fs::read_dir(&layout.features_dir()) else {
        return feature_matching_sessions;
    };

    for entry in entries {
        let Some(name) = entry.file_name() else {
            continue;
        };
        let Ok(feature_name) = crate::domain::name::FeatureName::new(name) else {
            continue;
        };
        let Ok(Some(feature)) = Feature::read(layout, &feature_name) else {
            continue;
        };
        let feature_dir = canonicalize_lenient(&layout.feature_dir(&feature_name));
        let sessions_dir = canonicalize_lenient(&layout.feature_sessions_dir(&feature_name));

        // Check if target is inside a specific session view dir under this feature
        let mut matched_specific_session = false;
        if let Ok(sessions) = super::lookup::list_feature(layout, &feature_name) {
            for s in sessions {
                if s.state.is_some() {
                    let view = canonicalize_lenient(&s.view_dir);
                    if target == view || target.starts_with(&view) {
                        feature_matching_sessions.push((feature_name.clone(), s));
                        matched_specific_session = true;
                        break;
                    }
                }
            }
        }

        if !matched_specific_session {
            let target_in_feature_dir =
                target.starts_with(&feature_dir) && !target.starts_with(&sessions_dir);
            let target_in_promoted_worktree = feature.promotions.keys().any(|repo| {
                let wt = canonicalize_lenient(&layout.repo_worktree(repo, &feature.branch));
                target == wt || target.starts_with(&wt)
            });

            if (target_in_feature_dir || target_in_promoted_worktree)
                && let Ok(Some(session)) = super::lookup::most_recent(layout, &feature_name)
                && session.state.is_some()
            {
                feature_matching_sessions.push((feature_name.clone(), session));
            }
        }
    }

    feature_matching_sessions
}

fn match_discovery_sessions(
    layout: &Layout,
    target: &Utf8Path,
) -> Vec<crate::domain::session::SessionRef> {
    let mut discovery_sessions = super::lookup::list_discovery(layout).unwrap_or_default();
    discovery_sessions.retain(|s| s.state.is_some());

    let mut matching = Vec::new();
    for s in discovery_sessions {
        let view = canonicalize_lenient(&s.view_dir);
        if target == view || target.starts_with(&view) {
            matching.push(s);
        }
    }
    matching
}

fn resolve_session_writable_set(
    layout: &Layout,
    session: &crate::domain::session::SessionRef,
) -> Option<WritableSet> {
    let state = session.state.as_ref()?;
    let env = crate::action::session::env::SessionEnv::build(
        layout,
        &session.id,
        &session.view_dir,
        state.provider,
        state.feature.as_ref(),
    );
    resolve_writable_set(&env)
}

/// Resolve the authoritative writable set for a target path when cwd resolves
/// no session.
fn resolve_set_by_target(target: &Utf8Path) -> TargetResolution {
    let mut current = target;
    let existing_ancestor = loop {
        if current.exists() {
            break current;
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break current,
        }
    };

    let Ok(Some(layout)) = Layout::discover(existing_ancestor) else {
        return TargetResolution::None;
    };

    // 1. Check session-specific roots across all live sessions.
    let feature_matching_sessions = match_feature_sessions(&layout, target);
    let matching_discovery = match_discovery_sessions(&layout, target);

    // Deduplicate matching features
    let mut unique_features: std::collections::BTreeMap<
        String,
        crate::domain::session::SessionRef,
    > = std::collections::BTreeMap::new();
    for (feat, session) in feature_matching_sessions {
        unique_features.entry(feat.to_string()).or_insert(session);
    }

    let total_session_matches = unique_features.len() + matching_discovery.len();
    if total_session_matches > 1 {
        let mut names: Vec<String> = unique_features.into_keys().collect();
        for d in matching_discovery {
            names.push(d.id.to_string());
        }
        names.sort();
        return TargetResolution::Ambiguous(names);
    }

    if let Some((_, session)) = unique_features.into_iter().next()
        && let Some(set) = resolve_session_writable_set(&layout, &session)
    {
        return TargetResolution::Unique(set);
    }

    if let Some(session) = matching_discovery.into_iter().next()
        && let Some(set) = resolve_session_writable_set(&layout, &session)
    {
        return TargetResolution::Unique(set);
    }

    // 2. Target does not lie in any session-specific root.
    // Check shared hall space (HallRoot::allows or hall_sources).
    let hall_root = HallRoot::new(&layout);
    let is_hall_source = hall_sources(&layout).iter().any(|hs| within(target, hs));
    if hall_root.allows(target) || is_hall_source {
        let dummy_view = layout.root().to_path_buf();
        let set = WritableSet {
            view_dir: dummy_view,
            feature_dir: None,
            sessions_dir: None,
            worktrees: Vec::new(),
            hall_sources: hall_sources(&layout),
            hall: hall_root,
        };
        return TargetResolution::SharedHall(set);
    }

    TargetResolution::None
}

/// Run the guard: parse stdin JSON, resolve the session, decide, and
/// shape the output for the given provider.
pub fn guard(provider: Provider, stdin_json: &str) -> Result<GuardOutcome, Failure> {
    let (tool_request, cwd) = crate::providers::parse_tool_request(provider, stdin_json)?;

    let session_env = cwd
        .as_deref()
        .and_then(|cwd| crate::action::session::env::SessionEnv::resolve_by_cwd(cwd).ok())
        .flatten();
    let mut set = session_env.as_ref().and_then(resolve_writable_set);

    let target = tool_request
        .file_path
        .as_ref()
        .and_then(|fp| resolve_target(cwd.as_deref(), fp));

    let mut ambiguous_features = None;

    if set.is_none()
        && is_structured_write(&tool_request.tool)
        && let Some(target_path) = target.as_deref()
    {
        let is_relative = tool_request
            .file_path
            .as_ref()
            .is_some_and(|p| !p.is_absolute());
        if is_relative {
            // Relative writes whose cwd resolves no session are denied.
            // Do not resolve set by target for relative writes.
        } else {
            match resolve_set_by_target(target_path) {
                TargetResolution::Unique(s) | TargetResolution::SharedHall(s) => {
                    set = Some(s);
                }
                TargetResolution::Ambiguous(features) => {
                    ambiguous_features = Some(features);
                }
                TargetResolution::None => {}
            }
        }
    }

    let mut relative_denial = None;
    if set.is_none()
        && is_structured_write(&tool_request.tool)
        && let Some(file_path) = &tool_request.file_path
        && !file_path.is_absolute()
        && !has_uri_scheme(file_path.as_str())
    {
        relative_denial = Some(relative_no_session_reason(file_path, cwd.as_deref()));
    }

    let resolution = match (&set, ambiguous_features) {
        (Some(set), _) => Resolution::Resolved(set),
        (None, Some(features)) => Resolution::Ambiguous { features },
        (None, None) => Resolution::Unresolved {
            scratch_dirs: live_scratch_dirs(cwd.as_deref()),
        },
    };

    let decision = if let Some(reason) = relative_denial {
        GuardDecision::Deny { reason }
    } else {
        decide(&resolution, &tool_request, target.as_deref())
    };
    if matches!(decision, GuardDecision::Allow)
        && is_graph_explore_tool(&tool_request.tool)
        && let Some(cwd) = cwd.as_deref()
    {
        record_graph_call_at(
            cwd,
            session_env.as_ref(),
            std::env::var("IVAR_SESSION_ID").ok(),
        );
    }

    if let Some(pattern) = &tool_request.search_pattern
        && let Some(cwd) = cwd.as_deref()
    {
        record_search_miss_at(
            cwd,
            session_env.as_ref(),
            std::env::var("IVAR_SESSION_ID").ok(),
            pattern,
        );
    }

    Ok(crate::providers::render_decision(provider, &decision))
}

/// Claude Code spells MCP tools `mcp__<server>__<tool>`; OpenCode and OMP
/// join the server (named `…graph`) and tool with a single `_`.
fn is_graph_explore_tool(tool: &str) -> bool {
    tool == "graph_explore"
        || tool.ends_with("__graph_explore")
        || tool.ends_with("-graph_graph_explore")
        || tool.ends_with("_graph_graph_explore")
}

/// The MCP server runs once per hall and cannot tell which session called
/// it; the hook's payload cwd can, so the hook stamps the session.
fn record_graph_call_at(
    cwd: &Utf8Path,
    session_env: Option<&crate::action::session::env::SessionEnv>,
    ambient_session: Option<String>,
) {
    let Some(session) =
        crate::action::graph::session::session_key_for(session_env, ambient_session)
    else {
        return;
    };
    let layout = match session_env {
        Some(env) => Some(Layout::at(env.hall.clone())),
        None => Layout::discover(cwd).ok().flatten(),
    };
    let Some(layout) = layout else { return };
    let db_path = layout.ivar_dir().join("memory.db");
    let Ok(db) = crate::store::graph::db::GraphDb::open_for_usage(db_path.as_std_path()) else {
        return;
    };
    let _ = db.record_usage(&UsageEvent {
        command: "graph_explore".to_owned(),
        source: UsageSource::Hook,
        duration_ms: 0,
        result_count: None,
        error: false,
        session: Some(session),
        query: None,
    });
}

/// How long after a graph call a search counts as a follow-up rather than an
/// unrelated later search.
const FOLLOWUP_WINDOW_SECS: i64 = 120;

fn record_search_miss_at(
    cwd: &Utf8Path,
    session_env: Option<&crate::action::session::env::SessionEnv>,
    ambient_session: Option<String>,
    pattern: &str,
) {
    let Some(session) =
        crate::action::graph::session::session_key_for(session_env, ambient_session)
    else {
        return;
    };
    let layout = match session_env {
        Some(env) => Some(Layout::at(env.hall.clone())),
        None => Layout::discover(cwd).ok().flatten(),
    };
    if let Some(layout) = layout {
        record_search_miss(&layout, &session, pattern);
    }
}

/// Best-effort classification of one search-tool call as `skipped` or
/// `followup`. Every failure is swallowed: the guard's decision is already
/// made, and nothing here may change it or its exit code.
fn record_search_miss(layout: &Layout, session: &str, pattern: &str) {
    let db_path = layout.ivar_dir().join("memory.db");
    if !db_path.exists() {
        return;
    }
    let Ok(db) = crate::store::graph::db::GraphDb::open_for_usage(db_path.as_std_path()) else {
        return;
    };

    let miss = |kind, query| MissEvent {
        session: Some(session.to_owned()),
        kind,
        query,
        pattern: Some(pattern.to_owned()),
        reason: None,
    };
    let _ = match db.last_graph_call(session) {
        Ok(None) => db.record_miss(&miss(MissKind::Skipped, None)),
        Ok(Some(call))
            if crate::store::graph::db::types::now_timestamp() - call.ts
                <= FOLLOWUP_WINDOW_SECS
                && matches!(db.has_followup_for(&call), Ok(false)) =>
        {
            db.record_followup(&miss(MissKind::Followup, call.query.clone()), &call)
        }
        _ => Ok(()),
    };
}

/// Try to build a `WritableSet` from a resolved session env.
///
/// A session with no feature is a discovery session, not an unknown: it
/// resolves to a set holding the view dir, the canonical hall sources, and
/// the hall root outside `.ivar/`. Returning `None` there would disarm the
/// guard for that session.
fn resolve_writable_set(env: &crate::action::session::env::SessionEnv) -> Option<WritableSet> {
    let layout = Layout::discover(&env.view_dir).ok()??;
    let Some(feature_name) = env.feature.as_ref() else {
        return WritableSet::from_discovery(&layout, &env.view_dir).ok();
    };
    let feature = Feature::read(&layout, feature_name).ok()??;
    WritableSet::from_session(&layout, &feature, &env.view_dir).ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/guard.rs"]
mod tests;
