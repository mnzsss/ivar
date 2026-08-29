//! The canonical session environment.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
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
            out.push_str(&format!("export IVAR_FEATURE={}\n", feature.as_str()));
        }
        out
    }

    /// Render a flat JSON object keyed by environment variable names.
    #[must_use]
    pub fn render_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert(
            "IVAR_HALL".to_string(),
            serde_json::Value::String(self.hall.to_string()),
        );
        map.insert(
            "IVAR_SESSION_ID".to_string(),
            serde_json::Value::String(self.session_id.clone()),
        );
        map.insert(
            "IVAR_SESSION_PATH".to_string(),
            serde_json::Value::String(self.view_dir.to_string()),
        );
        map.insert(
            "IVAR_PROVIDER".to_string(),
            serde_json::Value::String(self.provider.id().to_string()),
        );
        if let Some(feature) = &self.feature {
            map.insert(
                "IVAR_FEATURE".to_string(),
                serde_json::Value::String(feature.to_string()),
            );
        }
        serde_json::Value::Object(map)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/env.rs"]
mod tests;
