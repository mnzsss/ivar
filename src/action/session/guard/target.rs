//! Resolving a writable set from a write's target path when the cwd resolves no session.

use super::decide::has_uri_scheme;
use super::set::{HallRoot, WritableSet, canonicalize_lenient, hall_sources, within};
use crate::domain::feature::Feature;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// Resolve the target path for a tool request: absolute paths are leniently
/// canonicalized, relative paths are joined to payload cwd and leniently
/// canonicalized, and absent cwd/target returns `None`. Targets with an RFC 3986
/// URI scheme (e.g. `xd://...`, `memory://...`) return `None` so they are not
/// treated as filesystem targets.
pub(super) fn resolve_target(cwd: Option<&Utf8Path>, file_path: &Utf8Path) -> Option<Utf8PathBuf> {
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
pub(super) enum TargetResolution {
    None,
    SharedHall(WritableSet),
    Unique(WritableSet),
    Ambiguous(Vec<String>),
}

/// Resolve the authoritative writable set for a target path when cwd resolves
/// no session.
pub(super) fn match_feature_sessions(
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
        if let Ok(sessions) = crate::action::session::lookup::list_feature(layout, &feature_name) {
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
                && let Ok(Some(session)) =
                    crate::action::session::lookup::most_recent(layout, &feature_name)
                && session.state.is_some()
            {
                feature_matching_sessions.push((feature_name.clone(), session));
            }
        }
    }

    feature_matching_sessions
}

pub(super) fn match_discovery_sessions(
    layout: &Layout,
    target: &Utf8Path,
) -> Vec<crate::domain::session::SessionRef> {
    let mut discovery_sessions =
        crate::action::session::lookup::list_discovery(layout).unwrap_or_default();
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

pub(super) fn resolve_session_writable_set(
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
pub(super) fn resolve_set_by_target(target: &Utf8Path) -> TargetResolution {
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
            descendant_feature_dirs: Vec::new(),
            descendant_sessions_dirs: Vec::new(),
            descendant_worktrees: Vec::new(),
            hall_sources: hall_sources(&layout),
            hall: hall_root,
        };
        return TargetResolution::SharedHall(set);
    }

    TargetResolution::None
}

/// Try to build a `WritableSet` from a resolved session env.
///
/// A session with no feature is a discovery session, not an unknown: it
/// resolves to a set holding the view dir, the canonical hall sources, and
/// the hall root outside `.ivar/`. Returning `None` there would disarm the
/// guard for that session.
pub(super) fn resolve_writable_set(
    env: &crate::action::session::env::SessionEnv,
) -> Option<WritableSet> {
    let layout = Layout::discover(&env.view_dir).ok()??;
    let Some(feature_name) = env.feature.as_ref() else {
        return WritableSet::from_discovery(&layout, &env.view_dir).ok();
    };
    let feature = Feature::read(&layout, feature_name).ok()??;
    WritableSet::from_session(&layout, &feature, &env.view_dir).ok()
}
