//! The canonical session environment.

use camino::{Utf8Path, Utf8PathBuf};
use std::fmt::Write as _;

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::infra::fs;
use crate::infra::proc::Command;
use crate::store::layout::Layout;

/// The session environment variable delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnv {
    pub hall: Utf8PathBuf,
    pub session_id: String,
    pub view_dir: Utf8PathBuf,
    pub provider: Provider,
    pub feature: Option<FeatureName>,
}

impl SessionEnv {
    /// Construct a `SessionEnv` pure value for a given layout, session, view dir, provider, and optional feature.
    #[must_use]
    pub fn build(
        layout: &Layout,
        session_id: &SessionId,
        view_dir: &Utf8Path,
        provider: Provider,
        feature: Option<&FeatureName>,
    ) -> Self {
        Self {
            hall: layout.root().to_path_buf(),
            session_id: session_id.to_string(),
            view_dir: view_dir.to_path_buf(),
            provider,
            feature: feature.cloned(),
        }
    }

    /// Apply these session environment variables to a `proc::Command`.
    #[must_use]
    pub fn apply(&self, mut command: Command) -> Command {
        command = command
            .env("IVAR_HALL", self.hall.as_str())
            .env("IVAR_SESSION_ID", &self.session_id)
            .env("IVAR_SESSION_PATH", self.view_dir.as_str())
            .env("IVAR_PROVIDER", self.provider.id());
        if let Some(feature) = &self.feature {
            command = command.env("IVAR_FEATURE", feature.as_str());
        }
        command
    }

    /// Render shell `export VAR=val` statements for human/shell output.
    #[must_use]
    pub fn render_shell(&self) -> String {
        let mut out = format!(
            "export IVAR_HALL={}\nexport IVAR_SESSION_ID={}\nexport IVAR_SESSION_PATH={}\nexport IVAR_PROVIDER={}\n",
            self.hall,
            self.session_id,
            self.view_dir,
            self.provider.id()
        );
        if let Some(feature) = &self.feature {
            let _ = writeln!(out, "export IVAR_FEATURE={}", feature.as_str());
        }
        out
    }

    /// Render a flat JSON object keyed by environment variable names.
    #[must_use]
    pub fn render_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert(
            "IVAR_HALL".to_owned(),
            serde_json::Value::String(self.hall.to_string()),
        );
        map.insert(
            "IVAR_SESSION_ID".to_owned(),
            serde_json::Value::String(self.session_id.clone()),
        );
        map.insert(
            "IVAR_SESSION_PATH".to_owned(),
            serde_json::Value::String(self.view_dir.to_string()),
        );
        map.insert(
            "IVAR_PROVIDER".to_owned(),
            serde_json::Value::String(self.provider.id().to_owned()),
        );
        if let Some(feature) = &self.feature {
            map.insert(
                "IVAR_FEATURE".to_owned(),
                serde_json::Value::String(feature.to_string()),
            );
        }
        serde_json::Value::Object(map)
    }
    /// The variable names present in this environment, in render order.
    #[must_use]
    pub fn keys(&self) -> Vec<&'static str> {
        let mut k = vec![
            "IVAR_HALL",
            "IVAR_SESSION_ID",
            "IVAR_SESSION_PATH",
            "IVAR_PROVIDER",
        ];
        if self.feature.is_some() {
            k.push("IVAR_FEATURE");
        }
        k
    }

    /// Walk up from `start` looking for a `state.json` inside a view directory;
    /// failing that, the newest session of the feature whose promoted worktree
    /// contains `start`.
    ///
    /// Reads NO environment variables — resolution is pure disk walk-up.
    pub fn resolve_by_cwd(start: &Utf8Path) -> Result<Option<Self>, crate::error::Failure> {
        Self::resolve_for_agent(start, None)
    }

    /// `resolve_by_cwd`, except that when the walk-up finds no view dir and
    /// `ambient` (the agent's `IVAR_SESSION_ID`) names a session of the hall that
    /// `cwd` belongs to, that session wins over the promoted-worktree fallback.
    ///
    /// The fallback assumes whoever stands in a feature's worktree runs under
    /// that feature's newest session. A subfeature agent whose view links its
    /// parent's worktree stands there too, and must keep its own session — and
    /// its own writable set. Reads no environment variable itself: the caller
    /// passes `ambient`. An id that is not exactly a live session of this hall
    /// is ignored.
    pub fn resolve_for_agent(
        cwd: &Utf8Path,
        ambient: Option<&str>,
    ) -> Result<Option<Self>, crate::error::Failure> {
        Self::resolve(cwd, None, ambient)
    }

    /// `resolve_for_agent` for a write to `target` (already canonical). When
    /// the cwd lies in no view dir, the live session whose view dir holds
    /// `target` wins over the ambient id: a write that names its session
    /// lands in it. The cwd's view dir still wins over the target, so a
    /// session never borrows another session's writable set, and a session
    /// of another hall never matches.
    pub fn resolve_for_write(
        cwd: &Utf8Path,
        target: &Utf8Path,
        ambient: Option<&str>,
    ) -> Result<Option<Self>, crate::error::Failure> {
        Self::resolve(cwd, Some(target), ambient)
    }

    fn resolve(
        cwd: &Utf8Path,
        target: Option<&Utf8Path>,
        ambient: Option<&str>,
    ) -> Result<Option<Self>, crate::error::Failure> {
        let current = match cwd.canonicalize_utf8() {
            Ok(path) => path,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(crate::error::Failure::blocked(
                    "fs.unresolvable",
                    format!("path `{cwd}` could not be resolved: {err}"),
                ));
            }
        };

        if let Some(env) = Self::from_view_dir_walk(&current)? {
            return Ok(Some(env));
        }

        let Some(layout) = Layout::discover(&current)? else {
            return Ok(None);
        };

        if let Some(target) = target
            && let Some(env) = Self::from_view_dir_walk(target).ok().flatten()
            && env.hall.as_path() == layout.root()
        {
            return Ok(Some(env));
        }

        if let Some(env) = ambient.and_then(|id| Self::from_ambient(&layout, id)) {
            return Ok(Some(env));
        }

        Self::from_promoted_worktree(&layout, &current)
    }

    /// The session whose view dir holds `current` (or an ancestor of it).
    fn from_view_dir_walk(current: &Utf8Path) -> Result<Option<Self>, crate::error::Failure> {
        let mut walk = current.to_path_buf();
        loop {
            if walk.join("state.json").is_file()
                && let Some(file_name) = walk.file_name()
                && let Ok(session_id) = SessionId::new(file_name)
                && let (Some(layout), Ok(Some(state))) =
                    (Layout::discover(&walk)?, SessionState::read(&walk))
            {
                let env = Self::build(
                    &layout,
                    &session_id,
                    &walk,
                    state.provider,
                    state.feature.as_ref(),
                );
                return Ok(Some(env));
            }

            match walk.parent() {
                Some(parent) => walk = parent.to_path_buf(),
                None => return Ok(None),
            }
        }
    }

    /// The live session of `layout` whose id is exactly `id`. A lookup failure
    /// (no match, ambiguous prefix, unreadable tree) or a prefix match is
    /// `None`: an ambient id only ever narrows resolution to a real session.
    fn from_ambient(layout: &Layout, id: &str) -> Option<Self> {
        let session = super::lookup::resolve(layout, Some(id), None).ok()?;
        if session.id.as_str() != id {
            return None;
        }
        let state = session.state.as_ref()?;
        Some(Self::build(
            layout,
            &session.id,
            &session.view_dir,
            state.provider,
            state.feature.as_ref(),
        ))
    }

    /// The newest session of the feature whose promoted worktree contains
    /// `current`.
    fn from_promoted_worktree(
        layout: &Layout,
        current: &Utf8Path,
    ) -> Result<Option<Self>, crate::error::Failure> {
        if !fs::is_dir(&layout.features_dir())? {
            return Ok(None);
        }

        for entry in fs::read_dir(&layout.features_dir())? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = FeatureName::new(name) else {
                continue;
            };
            let Some(feature) = Feature::read(layout, &feature_name)? else {
                continue;
            };

            let matches_worktree = feature.promotions.keys().any(|repo| {
                let wt = layout.repo_worktree(repo, &feature.branch);
                let canonical_wt = canonicalize_lenient(&wt);
                current == canonical_wt.as_path() || current.starts_with(&canonical_wt)
            });

            if !matches_worktree {
                continue;
            }

            // A feature accumulates sessions — every `session start` on it
            // adds one, and old ones outlive their agents. Demanding exactly
            // one would make this fallback dead on any feature worked on
            // twice, so take the most recent: the session an agent standing
            // in this worktree is running under.
            let Some(session) = super::lookup::most_recent(layout, &feature_name)? else {
                return Ok(None);
            };
            let Some(state) = session.state.as_ref() else {
                return Ok(None);
            };

            let env = Self::build(
                layout,
                &session.id,
                &session.view_dir,
                state.provider,
                state.feature.as_ref(),
            );
            return Ok(Some(env));
        }

        Ok(None)
    }
}

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

impl crate::error::WriteHuman for SessionEnv {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(w, "{}", self.render_shell())
    }
}

impl serde::Serialize for SessionEnv {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.render_json().serialize(serializer)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/env.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/session/env_contract.rs"]
mod contract_tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/session/env_cmd.rs"]
mod env_cmd_tests;
