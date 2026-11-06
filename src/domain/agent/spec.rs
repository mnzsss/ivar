//! An agent file's frontmatter and prompt, validated into an [`AgentSpec`].

use std::collections::BTreeMap;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use super::at::AtTime;
use super::cron::CronExpr;
use super::tool::Tool;
use crate::domain::name::FeatureName;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_TIMEOUT_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontmatter {
    pub schedule: Option<String>,
    pub at: Option<String>,
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
    #[serde(default)]
    pub catchup: bool,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
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
pub enum NotifyTarget {
    Webhook(String),
    Telegram { chat_id: String },
}

impl std::fmt::Display for NotifyTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Webhook(url) => f.write_str(url),
            Self::Telegram { chat_id } => write!(f, "telegram:{chat_id}"),
        }
    }
}

pub(crate) fn is_telegram_chat_id(id: &str) -> bool {
    if let Some(channel) = id.strip_prefix('@') {
        return (5..=32).contains(&channel.len())
            && channel.starts_with(|c: char| c.is_ascii_alphabetic())
            && channel
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn notify_target(value: &str) -> Result<NotifyTarget, SpecError> {
    if let Some(chat_id) = value.strip_prefix("telegram:") {
        return if is_telegram_chat_id(chat_id) {
            Ok(NotifyTarget::Telegram {
                chat_id: chat_id.to_owned(),
            })
        } else {
            Err(err(
                "notify",
                format!(
                    "`{chat_id}` is not a Telegram chat id; use a number like -100123 or @channel_name"
                ),
            ))
        };
    }
    if value.starts_with("https://") || value.starts_with("http://") {
        return Ok(NotifyTarget::Webhook(value.to_owned()));
    }
    Err(err(
        "notify",
        format!("`{value}` must be an http:// or https:// URL, or telegram:<chat_id>"),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritePath {
    Absolute(Utf8PathBuf),
    Home(Utf8PathBuf),
    Hall(Utf8PathBuf),
}

impl WritePath {
    /// # Errors
    ///
    /// When `raw` is empty, has a `..` segment or names another user's home.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.is_empty() || raw.split('/').any(|part| part == "..") {
            return Err(format!("`{raw}` must be a non-empty path without `..`"));
        }
        if raw.starts_with('~') && raw != "~" && !raw.starts_with("~/") {
            return Err(format!(
                "`{raw}` names another user's home; use `~/…` or an absolute path"
            ));
        }
        Ok(if raw == "~" {
            Self::Home(Utf8PathBuf::new())
        } else if let Some(rest) = raw.strip_prefix("~/") {
            Self::Home(rest.into())
        } else if raw.starts_with('/') {
            Self::Absolute(raw.into())
        } else {
            Self::Hall(raw.into())
        })
    }

    pub fn resolve(&self, home: &Utf8Path, hall: &Utf8Path) -> Utf8PathBuf {
        match self {
            Self::Absolute(path) => path.clone(),
            Self::Home(rel) => home.join(rel),
            Self::Hall(rel) => hall.join(rel),
        }
    }
}

/// When an agent runs: on every match of a cron expression, or once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timing {
    /// A recurring agent, from `schedule:`.
    Recurring(CronExpr),
    /// An Agent Task, from `at:`: runs once at this time.
    Once(AtTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub name: String,
    pub timing: Timing,
    pub access: Access,
    pub runtime: Runtime,
    pub network: Vec<String>,
    pub secrets: Vec<String>,
    pub launch: Launch,
    pub notify: Option<NotifyTarget>,
    pub timeout: Duration,
    pub workdir: Option<String>,
    pub catchup: bool,
    pub write: Vec<WritePath>,
    pub env: BTreeMap<String, String>,
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

/// Variables ivar sets so the tool stays on its private state and git has no credentials;
/// an agent's `env` overriding one would undo that confinement.
pub const RESERVED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "OPENCODE_CONFIG_CONTENT",
];

fn env_name_error(name: &str, secrets: &[String]) -> Option<String> {
    let well_formed = name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !well_formed {
        return Some(format!("`{name}` must match ^[A-Z_][A-Z0-9_]*$"));
    }
    if RESERVED_ENV.contains(&name) || name.starts_with("GIT_") {
        return Some(format!(
            "`{name}` is set by ivar to keep the run confined and cannot be overridden"
        ));
    }
    if secrets.iter().any(|secret| secret == name) {
        return Some(format!(
            "`{name}` is already in `secrets`; a secret's value belongs only in agents.env"
        ));
    }
    None
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

fn timing_of(fm: &AgentFrontmatter) -> Result<Timing, SpecError> {
    match (fm.schedule.as_deref(), fm.at.as_deref()) {
        (Some(_), Some(_)) => Err(err(
            "at",
            "`at` makes a one-shot Agent Task and cannot be combined with `schedule`; set one of them",
        )),
        (None, None) => Err(err(
            "schedule",
            "is required (or `at` for a one-shot Agent Task)",
        )),
        (Some(schedule), None) => CronExpr::parse(schedule)
            .map(Timing::Recurring)
            .map_err(|e| err("schedule", e.reason)),
        (None, Some(at)) => {
            if fm.catchup {
                return Err(err(
                    "catchup",
                    "an Agent Task with `at` always catches up after downtime; remove `catchup`",
                ));
            }
            AtTime::parse(at)
                .map(Timing::Once)
                .map_err(|reason| err("at", reason))
        }
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
        let timing = timing_of(&fm)?;
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
        } else if !fm.write.is_empty() {
            return Err(err(
                "write",
                "`write` adds Landlock roots to a `runtime: host` run; remove it or set `runtime: host`",
            ));
        } else if !fm.env.is_empty() {
            return Err(err(
                "env",
                "`env` is passed to a `runtime: host` run; remove it or set `runtime: host`",
            ));
        }
        let write = fm
            .write
            .iter()
            .map(|raw| WritePath::parse(raw))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|reason| err("write", reason))?;
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
        if let Some(reason) = fm
            .env
            .keys()
            .find_map(|name| env_name_error(name, &fm.secrets))
        {
            return Err(err("env", reason));
        }
        let notify = fm.notify.as_deref().map(notify_target).transpose()?;
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
            timing,
            access: fm.access,
            runtime: fm.runtime,
            network: fm.network,
            secrets: fm.secrets,
            launch,
            notify,
            timeout,
            workdir: fm.workdir,
            catchup: fm.catchup,
            write,
            env: fm.env,
            prompt: prompt.to_owned(),
        })
    }

    /// Whether this is a one-shot Agent Task rather than a recurring agent.
    pub const fn is_task(&self) -> bool {
        matches!(self.timing, Timing::Once(_))
    }

    /// The cron schedule of a recurring agent.
    pub const fn schedule(&self) -> Option<&CronExpr> {
        match &self.timing {
            Timing::Recurring(cron) => Some(cron),
            Timing::Once(_) => None,
        }
    }

    /// The time of an Agent Task.
    pub const fn at(&self) -> Option<AtTime> {
        match self.timing {
            Timing::Once(at) => Some(at),
            Timing::Recurring(_) => None,
        }
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
