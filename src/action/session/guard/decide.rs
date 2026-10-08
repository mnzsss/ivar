//! The decision: allow or deny a tool request against a resolution, with denial guidance.

use super::set::{WritableSet, canonicalize_lenient};
use crate::domain::feature::Feature;
pub(super) use crate::domain::guard::{GuardDecision, ToolRequest};
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// Check whether a path string starts with an RFC 3986 URI scheme (`<scheme>://`).
/// Schemes match `^[a-zA-Z][a-zA-Z0-9+.-]*://`.
pub(super) fn has_uri_scheme(s: &str) -> bool {
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
    Unresolved {
        scoped_scratch_dirs: Vec<Utf8PathBuf>,
        live_count: usize,
    },
    Ambiguous {
        features: Vec<String>,
    },
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
    targets: &[Utf8PathBuf],
) -> GuardDecision {
    if !req.writes {
        return GuardDecision::Allow;
    }
    if !req.targets.is_empty() && req.targets.iter().all(|p| has_uri_scheme(p.as_str())) {
        return GuardDecision::Allow;
    }
    match resolution {
        Resolution::Resolved(set) => {
            if targets.is_empty() {
                let guidance = denial_guidance(Some(set), None, None);
                return GuardDecision::Deny {
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
                };
            }
            if let Some(denied_target) = targets.iter().find(|t| !set.allows(t)) {
                let original = req
                    .targets
                    .iter()
                    .find(|orig| {
                        orig.as_str() == denied_target.as_str()
                            || denied_target.ends_with(orig.as_path())
                    })
                    .map(|p| p.as_path());
                let guidance = denial_guidance(Some(set), Some(denied_target), original);
                GuardDecision::Deny {
                    reason: format!(
                        "`{denied_target}` is outside the writable set; writable set: {}; {guidance}",
                        std::iter::once(set.view_dir().to_string())
                            .chain(set.feature_dir.as_ref().map(|f| f.to_string()))
                            .chain(set.worktrees.iter().map(|w| w.to_string()))
                            .chain(set.hall_sources.iter().map(|h| h.to_string()))
                            .chain([set.hall.to_string()])
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                }
            } else {
                GuardDecision::Allow
            }
        }
        Resolution::Unresolved {
            scoped_scratch_dirs,
            live_count,
        } => GuardDecision::Deny {
            reason: unresolved_reason(scoped_scratch_dirs, *live_count),
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
pub(super) fn denial_guidance(
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

pub(super) fn classify_layout_path_denial(
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
pub(super) fn unresolved_reason(scoped: &[Utf8PathBuf], live_count: usize) -> String {
    const SENTENCE: &str = "no ivar session resolves from the cwd or the target path";
    if !scoped.is_empty() {
        format!(
            "{SENTENCE}; temporary files belong in {}",
            scoped
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        match live_count {
            0 => format!("{SENTENCE}; this hall has no live session"),
            1 => format!("{SENTENCE}; this hall has 1 live session"),
            n => format!("{SENTENCE}; this hall has {n} live sessions"),
        }
    }
}

pub(super) fn feature_scratch_dirs(layout: &Layout, target: &Utf8Path) -> Vec<Utf8PathBuf> {
    let target = canonicalize_lenient(target);
    let Ok(entries) = crate::infra::fs::read_dir(&layout.features_dir()) else {
        return Vec::new();
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
        let in_feature_dir = target.starts_with(&feature_dir);
        let in_promoted_wt = feature.promotions.keys().any(|repo| {
            let wt = canonicalize_lenient(&layout.repo_worktree(repo, &feature.branch));
            target.starts_with(&wt)
        });

        if (in_feature_dir || in_promoted_wt)
            && let Ok(sessions) =
                crate::action::session::lookup::list_feature(layout, &feature_name)
        {
            let live_scratches: Vec<Utf8PathBuf> = sessions
                .into_iter()
                .filter(|s| s.state.is_some())
                .map(|s| Layout::session_scratch(&s.view_dir))
                .collect();
            if !live_scratches.is_empty() {
                return live_scratches;
            }
        }
    }

    Vec::new()
}

pub(super) fn relative_no_session_reason(
    relative_path: &Utf8Path,
    cwd: Option<&Utf8Path>,
) -> String {
    match cwd {
        Some(cwd) => format!(
            "relative path `{relative_path}` was resolved against `{cwd}`, which belongs to no ivar session; use an absolute path inside your session's worktree or view dir"
        ),
        None => format!(
            "relative path `{relative_path}` was resolved without a cwd, which belongs to no ivar session; use an absolute path inside your session's worktree or view dir"
        ),
    }
}
