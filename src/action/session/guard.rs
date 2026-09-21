//! The session write guard: determines which files a session may write.
//!
use crate::domain::feature::Feature;
pub use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// The set of paths a session is allowed to write into: its view dir, its
/// feature directory (for feature sessions), the worktrees of promoted repos,
/// and the hall's canonical sources (HALL.md, .ivar/skills, .ivar/skills-local).
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    view_dir: Utf8PathBuf,
    feature_dir: Option<Utf8PathBuf>,
    sessions_dir: Option<Utf8PathBuf>,
    worktrees: Vec<Utf8PathBuf>,
    hall_sources: Vec<Utf8PathBuf>,
}

/// Leniently canonicalise `path`. If canonicalisation fails (e.g. for a
/// file that does not exist yet), walk to the nearest existing ancestor,
/// canonicalise it, and append the remaining non-existent components,
/// falling back to the raw path if no ancestor canonicalises.
fn canonicalize_lenient(path: &Utf8Path) -> Utf8PathBuf {
    let mut existing = path;
    let mut tail = Vec::new();
    while !existing.exists() {
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
        layout.root().join("HALL.md"),
        layout.hall_skills(),
        layout.hall_skills_local(),
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
        })
    }

    /// Build the writable set for a discovery session: the view dir and
    /// canonical hall sources.
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
        })
    }

    /// Whether `path` is inside the view dir, canonical hall sources, feature
    /// directory when applicable, or one of the promoted worktrees.
    /// The input path is canonicalised (with parent fallback for not-yet-existing
    /// files) so symlinks cannot escape the set on platforms like macOS where
    /// `/tmp` or `/var` are symlinks.
    pub(crate) fn allows(&self, path: &Utf8Path) -> bool {
        let canonical = canonicalize_lenient(path);
        if canonical.starts_with(&self.view_dir) {
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
            .is_some_and(|fd| canonical.starts_with(fd))
        {
            return true;
        }
        if self.worktrees.iter().any(|wt| canonical.starts_with(wt)) {
            return true;
        }
        self.hall_sources.iter().any(|root| {
            if root.file_name() == Some("HALL.md") {
                canonical == *root
            } else {
                canonical.starts_with(root)
            }
        })
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
    /// feature dir (if present), and every promoted repo worktree. Note that
    /// `sessions_dir` is an exclusion boundary under `feature_dir` and is not a root.
    pub(crate) fn roots(&self) -> Vec<&Utf8Path> {
        let mut roots = Vec::with_capacity(
            1 + usize::from(self.feature_dir.is_some())
                + self.worktrees.len()
                + self.hall_sources.len(),
        );
        roots.push(self.view_dir.as_path());
        if let Some(feature_dir) = &self.feature_dir {
            roots.push(feature_dir.as_path());
        }
        roots.extend(self.worktrees.iter().map(Utf8PathBuf::as_path));
        roots.extend(self.hall_sources.iter().map(Utf8PathBuf::as_path));
        roots
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
            view_dir,
            feature_dir,
            sessions_dir,
            worktrees,
            hall_sources: Vec::new(),
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
}

/// Decide whether a tool request is allowed inside the session.
///
/// Structured write tools are checked against the writable set; everything
/// else is allowed. Shell is not classified here — it is a separate layer.
///
/// An absent set means neither the cwd nor the target resolved a session, so
/// the denial names both: the caller's next move is to check where the target
/// lives, not only where the agent stands.
pub(crate) fn decide(resolution: &Resolution<'_>, req: &ToolRequest) -> GuardDecision {
    if !is_structured_write(&req.tool) {
        return GuardDecision::Allow;
    }
    if let Some(path) = &req.file_path
        && has_uri_scheme(path.as_str())
    {
        return GuardDecision::Allow;
    }
    match resolution {
        Resolution::Resolved(set) => match &req.file_path {
            Some(path) if set.allows(path) => GuardDecision::Allow,
            _ => GuardDecision::Deny {
                reason: format!(
                    "writable set: {}; temporary files belong in {}",
                    std::iter::once(set.view_dir().to_string())
                        .chain(set.feature_dir.as_ref().map(|f| f.to_string()))
                        .chain(set.worktrees.iter().map(|w| w.to_string()))
                        .chain(set.hall_sources.iter().map(|h| h.to_string()))
                        .collect::<Vec<_>>()
                        .join(", "),
                    set.scratch_dir(),
                ),
            },
        },
        Resolution::Unresolved { scratch_dirs } => GuardDecision::Deny {
            reason: unresolved_reason(scratch_dirs),
        },
    }
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

/// Resolve the target path for a tool request: absolute paths are returned
/// as-is, relative paths are joined to payload cwd, and absent cwd/target
/// returns `None`. Targets with an RFC 3986 URI scheme (e.g. `xd://...`, `memory://...`)
/// return `None` so they are not treated as filesystem targets.
fn resolve_target(cwd: Option<&Utf8Path>, file_path: &Utf8Path) -> Option<Utf8PathBuf> {
    if has_uri_scheme(file_path.as_str()) {
        None
    } else if file_path.is_absolute() {
        Some(file_path.to_path_buf())
    } else {
        cwd.map(|base| base.join(file_path))
    }
}

/// Resolve the authoritative writable set for a target path when cwd resolves
/// no session. Discovers layout from the target's nearest existing ancestor,
/// enumerates all live sessions, builds each writable set, keeps those that
/// allow `target`, and picks the one with the greatest `started_at` timestamp.
fn resolve_set_by_target(target: &Utf8Path) -> Option<WritableSet> {
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
    let mut sessions = super::lookup::list_discovery(&layout).ok()?;
    if let Ok(entries) = crate::infra::fs::read_dir(&layout.features_dir()) {
        for entry in entries {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = crate::domain::name::FeatureName::new(name) else {
                continue;
            };
            if let Ok(Some(session)) = super::lookup::most_recent(&layout, &feature_name) {
                sessions.push(session);
            }
        }
    }

    let mut candidates = Vec::new();
    for session in sessions {
        let Some(state) = session.state.as_ref() else {
            continue;
        };
        let env = crate::action::session::env::SessionEnv::build(
            &layout,
            &session.id,
            &session.view_dir,
            state.provider,
            state.feature.as_ref(),
        );
        let Some(set) = resolve_writable_set(&env) else {
            continue;
        };
        if set.allows(target) {
            candidates.push((state.started_at.clone(), set));
        }
    }

    // Sort descending by started_at; stable sort preserves enumeration order on ties
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.into_iter().next().map(|(_, set)| set)
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

    if set.is_none()
        && is_structured_write(&tool_request.tool)
        && let Some(file_path) = &tool_request.file_path
        && let Some(target) = resolve_target(cwd.as_deref(), file_path)
    {
        set = resolve_set_by_target(&target);
    }

    let resolution = match &set {
        Some(set) => Resolution::Resolved(set),
        // Only a structured write reaches a denial here, and only a denial
        // needs the list — so nothing else pays for the walk.
        None if is_structured_write(&tool_request.tool) => Resolution::Unresolved {
            scratch_dirs: live_scratch_dirs(cwd.as_deref()),
        },
        None => Resolution::Unresolved {
            scratch_dirs: Vec::new(),
        },
    };

    let decision = decide(&resolution, &tool_request);

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
    use crate::domain::graph::{MissEvent, MissKind};

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
    let event = match db.last_graph_call(session) {
        Ok(None) => Some(miss(MissKind::Skipped, None)),
        Ok(Some((ts, query)))
            if crate::store::graph::db::types::now_timestamp() - ts <= FOLLOWUP_WINDOW_SECS
                && matches!(db.last_miss_since(session, ts), Ok(false)) =>
        {
            Some(miss(MissKind::Followup, query))
        }
        _ => None,
    };

    if let Some(event) = event {
        let _ = db.record_miss(&event);
    }
}

/// Try to build a `WritableSet` from a resolved session env.
///
/// A session with no feature is a discovery session, not an unknown: it
/// resolves to a set holding the view dir alone. Returning `None` there would
/// disarm the guard in the one session that may write nothing.
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
