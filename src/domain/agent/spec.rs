//! An agent file's frontmatter and prompt, validated into an [`AgentSpec`].

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::cron::CronExpr;
use super::tool::Tool;
use crate::domain::name::FeatureName;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_TIMEOUT_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontmatter {
    pub schedule: Option<String>,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    pub tool: Option<String>,
    pub model: Option<String>,
    pub image: Option<String>,
    pub command: Option<Vec<String>>,
    pub notify: Option<String>,
    pub timeout: Option<String>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Read,
    Feature,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    #[default]
    Docker,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Tool { tool: Tool, model: Option<String> },
    Image { image: String, command: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub name: String,
    pub schedule: CronExpr,
    pub access: Access,
    pub runtime: Runtime,
    pub network: Vec<String>,
    pub secrets: Vec<String>,
    pub launch: Launch,
    pub notify: Option<String>,
    pub timeout: Duration,
    pub workdir: Option<String>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    pub field: &'static str,
    pub reason: String,
}

fn err(field: &'static str, reason: impl Into<String>) -> SpecError {
    SpecError {
        field,
        reason: reason.into(),
    }
}

fn is_host(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

fn is_secret_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn launch_of(fm: &AgentFrontmatter) -> Result<Launch, SpecError> {
    match (&fm.tool, &fm.image, &fm.command) {
        (Some(_), Some(_), _) => Err(err("image", "`image` replaces `tool`; set one of them")),
        (Some(tool), None, _) => Ok(Launch::Tool {
            tool: tool.parse().map_err(|e: String| err("tool", e))?,
            model: fm.model.clone(),
        }),
        (None, Some(image), Some(command)) if !command.is_empty() => Ok(Launch::Image {
            image: image.clone(),
            command: command.clone(),
        }),
        (None, Some(_), _) => Err(err("command", "`image` needs a non-empty `command`")),
        (None, None, _) => Err(err(
            "tool",
            "set `tool` (codex, claude, opencode) or `image` with `command`",
        )),
    }
}

impl AgentSpec {
    /// # Errors
    ///
    /// Returns a [`SpecError`] naming the first invalid field.
    pub fn from_parts(name: &str, fm: AgentFrontmatter, prompt: &str) -> Result<Self, SpecError> {
        FeatureName::new(format!("agent-{name}")).map_err(|e| {
            err(
                "name",
                format!("`{name}` cannot name feature `agent-{name}`: {e}"),
            )
        })?;
        let schedule = CronExpr::parse(
            fm.schedule
                .as_deref()
                .ok_or_else(|| err("schedule", "is required"))?,
        )
        .map_err(|e| err("schedule", e.reason))?;
        if fm.runtime == Runtime::Host {
            if fm.image.is_some() || fm.command.is_some() {
                return Err(err(
                    "runtime",
                    "`runtime: host` runs the installed `tool`; `image` and `command` need `runtime: docker`",
                ));
            }
            if !fm.network.is_empty() {
                return Err(err(
                    "network",
                    "`runtime: host` cannot enforce a `network` allowlist; remove `network` or use `runtime: docker`",
                ));
            }
        }
        let launch = launch_of(&fm)?;
        if let Some(bad) = fm.network.iter().find(|d| !is_host(d)) {
            return Err(err(
                "network",
                format!("`{bad}` is not a bare hostname (no scheme, path or port)"),
            ));
        }
        if let Some(bad) = fm.secrets.iter().find(|n| !is_secret_name(n)) {
            return Err(err(
                "secrets",
                format!("`{bad}` must match ^[A-Z][A-Z0-9_]*$"),
            ));
        }
        if let Some(url) = fm.notify.as_deref()
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(err(
                "notify",
                format!("`{url}` must be an http:// or https:// URL"),
            ));
        }
        if let Some(dir) = fm.workdir.as_deref()
            && (dir.is_empty() || dir.starts_with('/') || dir.split('/').any(|part| part == ".."))
        {
            return Err(err(
                "workdir",
                format!("`{dir}` must be a relative path inside the hall's repos, without `..`"),
            ));
        }
        if fm.access == Access::Feature && fm.workdir.is_none() {
            return Err(err(
                "workdir",
                "`access: feature` needs `workdir` naming the repo to change",
            ));
        }
        let timeout = fm
            .timeout
            .as_deref()
            .map(parse_timeout)
            .transpose()
            .map_err(|e| err("timeout", e))?
            .unwrap_or(DEFAULT_TIMEOUT);
        Ok(Self {
            name: name.to_owned(),
            schedule,
            access: fm.access,
            runtime: fm.runtime,
            network: fm.network,
            secrets: fm.secrets,
            launch,
            notify: fm.notify,
            timeout,
            workdir: fm.workdir,
            prompt: prompt.to_owned(),
        })
    }

    pub fn feature_name(&self) -> String {
        format!("agent-{}", self.name)
    }

    pub fn workdir_repo(&self) -> Option<&str> {
        self.workdir.as_deref()?.split('/').next()
    }
}

/// # Errors
///
/// Returns a reason when `text` is not `<n>s`, `<n>m` or `<n>h` between 1s and 24h.
pub fn parse_timeout(text: &str) -> Result<Duration, String> {
    let malformed = || format!("`{text}` is not a duration like 90s, 45m or 2h");
    let out_of_range = || format!("`{text}` must be between 1s and 24h");
    let (digits, unit_secs) = [("s", 1u64), ("m", 60), ("h", 3600)]
        .into_iter()
        .find_map(|(suffix, secs)| text.strip_suffix(suffix).map(|d| (d, secs)))
        .ok_or_else(malformed)?;
    let n: u64 = digits.parse().map_err(|_| malformed())?;
    let secs = n.checked_mul(unit_secs).ok_or_else(out_of_range)?;
    if secs == 0 || secs > MAX_TIMEOUT_SECS {
        return Err(out_of_range());
    }
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/spec.rs"]
mod tests;
