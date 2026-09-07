//! The session write guard: determines which files a session may write.
//!
use crate::domain::feature::Feature;
pub use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// The set of paths a session is allowed to write into: its view dir, its
/// feature directory (for feature sessions), plus the worktrees of promoted repos.
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    view_dir: Utf8PathBuf,
    feature_dir: Option<Utf8PathBuf>,
    sessions_dir: Option<Utf8PathBuf>,
    worktrees: Vec<Utf8PathBuf>,
}

/// Leniently canonicalise `path`. If canonicalisation fails (e.g. for a
/// file that does not exist yet), try canonicalising its parent and appending
/// the file name, falling back to the raw path if parent canonicalisation also
/// fails.
fn canonicalize_lenient(path: &Utf8Path) -> Utf8PathBuf {
    if let Ok(canonical) = path.canonicalize_utf8() {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name())
        && let Ok(canonical_parent) = parent.canonicalize_utf8()
    {
        return canonical_parent.join(file_name);
    }
    path.to_path_buf()
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
        })
    }

    /// Build the writable set for a discovery session: the view dir and
    /// nothing else.
    ///
    /// A discovery session binds no feature and promotes no repo, so the set is
    /// empty by construction — not absent. The difference is the whole point:
    /// an absent set once meant "the guard has nothing to say", which left every
    /// read-only worktree mounted under the view dir writable.
    pub(crate) fn from_discovery(view_dir: &Utf8Path) -> Result<Self, Failure> {
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
        })
    }

    /// Whether `path` is inside the view dir or one of the promoted worktrees.
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
        self.worktrees.iter().any(|wt| canonical.starts_with(wt))
    }

    /// The view dir — the canonical root of this set.
    pub(crate) fn view_dir(&self) -> &Utf8Path {
        &self.view_dir
    }

    /// Return the write-allowed root paths: view dir, feature dir (if present),
    /// and every promoted repo worktree. Note that `sessions_dir` is an exclusion
    /// boundary under `feature_dir` and is not a root.
    #[allow(dead_code)]
    pub(crate) fn roots(&self) -> Vec<&Utf8Path> {
        let mut roots =
            Vec::with_capacity(1 + usize::from(self.feature_dir.is_some()) + self.worktrees.len());
        roots.push(self.view_dir.as_path());
        if let Some(feature_dir) = &self.feature_dir {
            roots.push(feature_dir.as_path());
        }
        for wt in &self.worktrees {
            roots.push(wt.as_path());
        }
        roots
    }

    /// Build a `WritableSet` from explicit parts. Test-only.
    #[cfg(test)]
    pub(crate) fn from_parts(
        view_dir: Utf8PathBuf,
        feature_dir: Option<Utf8PathBuf>,
        worktrees: Vec<Utf8PathBuf>,
    ) -> Self {
        let view_dir = canonicalize_lenient(&view_dir);
        let sessions_dir = feature_dir
            .as_ref()
            .map(|fd| canonicalize_lenient(&fd.join("sessions")));
        let feature_dir = feature_dir.map(|fd| canonicalize_lenient(&fd));
        let worktrees = worktrees.iter().map(|w| canonicalize_lenient(w)).collect();
        Self {
            view_dir,
            feature_dir,
            sessions_dir,
            worktrees,
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

/// Decide whether a tool request is allowed inside the session.
///
/// Structured write tools are checked against the writable set; everything
/// else is allowed. Shell is not classified here — it is a separate layer.
///
/// An absent set means neither the cwd nor the target resolved a session, so
/// the denial names both: the caller's next move is to check where the target
/// lives, not only where the agent stands.
pub(crate) fn decide(set: Option<&WritableSet>, req: &ToolRequest) -> GuardDecision {
    if !is_structured_write(&req.tool) {
        return GuardDecision::Allow;
    }
    match (set, &req.file_path) {
        (Some(set), Some(path)) if set.allows(path) => GuardDecision::Allow,
        (Some(set), _) => GuardDecision::Deny {
            reason: format!(
                "writable set: {}",
                std::iter::once(set.view_dir().to_string())
                    .chain(set.feature_dir.as_ref().map(|f| f.to_string()))
                    .chain(set.worktrees.iter().map(|w| w.to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
        (None, _) => GuardDecision::Deny {
            reason: "no ivar session resolves from the cwd or the target path".into(),
        },
    }
}

/// Resolve the target path for a tool request: absolute paths are returned
/// as-is, relative paths are joined to payload cwd, and absent cwd/target
/// returns `None`.
fn resolve_target(cwd: Option<&Utf8Path>, file_path: &Utf8Path) -> Option<Utf8PathBuf> {
    if file_path.is_absolute() {
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

    let mut set = cwd
        .as_deref()
        .and_then(|cwd| crate::action::session::env::SessionEnv::resolve_by_cwd(cwd).ok())
        .flatten()
        .and_then(|env| resolve_writable_set(&env));

    if set.is_none()
        && is_structured_write(&tool_request.tool)
        && let Some(file_path) = &tool_request.file_path
        && let Some(target) = resolve_target(cwd.as_deref(), file_path)
    {
        set = resolve_set_by_target(&target);
    }

    let decision = decide(set.as_ref(), &tool_request);

    Ok(crate::providers::render_decision(provider, &decision))
}

/// Try to build a `WritableSet` from a resolved session env.
///
/// A session with no feature is a discovery session, not an unknown: it
/// resolves to a set holding the view dir alone. Returning `None` there would
/// disarm the guard in the one session that may write nothing.
fn resolve_writable_set(env: &crate::action::session::env::SessionEnv) -> Option<WritableSet> {
    let Some(feature_name) = env.feature.as_ref() else {
        return WritableSet::from_discovery(&env.view_dir).ok();
    };
    let layout = Layout::discover(&env.view_dir).ok()??;
    let feature = Feature::read(&layout, feature_name).ok()??;
    WritableSet::from_session(&layout, &feature, &env.view_dir).ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/guard.rs"]
mod tests;
