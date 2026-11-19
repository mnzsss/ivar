//! The check before a `runtime: host` run: is the agent's tool installed,
//! answering, and logged in? A run that fails it never starts; it is recorded
//! `blocked` with what to fix, and `ivar agent resume` runs it again.
//!
//! A `runtime: docker` run is not checked: its tool lives in the image and its
//! login is whatever `secrets` carry, neither of which the host can see.

use std::ffi::OsStr;
use std::time::Duration;

use camino::Utf8Path;

use super::HostDeps;
use super::host::is_login;
use crate::action::agent::requirements::find_on_path;
use crate::domain::agent::spec::{AgentSpec, Launch, Runtime};
use crate::domain::agent::tool::Tool;
use crate::error::Failure;
use crate::infra::fs;
use crate::infra::proc::{self, Command, Finished};

/// The receipt's reason when the tool is missing or does not answer.
pub(crate) const UNAVAILABLE: &str = "agent.provider_unavailable";
/// The receipt's reason when the tool has no usable login.
pub(crate) const AUTH: &str = "agent.provider_auth";

const PROBE_DEADLINE: Duration = Duration::from_secs(10);

/// What asking the tool itself found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolCheck {
    Ready,
    Missing,
    Broken(String),
}

/// Why a run did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Block {
    pub code: &'static str,
    /// The short label: `provider unavailable` or `auth error`.
    pub reason: &'static str,
    pub detail: String,
}

impl Block {
    /// The receipt's output: the code, the label, the detail and the way out.
    pub(super) fn message(&self, agent: &str) -> String {
        format!(
            "{}: {}: {}. Fix it, then run `ivar agent resume {agent}`.",
            self.code,
            self.reason,
            self.detail.trim_end_matches('.')
        )
    }
}

/// Finds `tool` on the run's own `PATH` and has it print its version, so a
/// broken install is told from a missing one. `scratch` holds the log.
pub(crate) fn probe_tool(tool: Tool, env: &[(String, String)], scratch: &Utf8Path) -> ToolCheck {
    let value = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let Some(exe) = value("PATH").and_then(|path| find_on_path(tool.as_str(), OsStr::new(path)))
    else {
        return ToolCheck::Missing;
    };
    let log = scratch.join("preflight.log");
    if let Err(error) = fs::ensure_dir(scratch) {
        return ToolCheck::Broken(Failure::from(error).what);
    }
    let mut command = Command::new(exe.to_string_lossy())
        .arg("--version")
        .env_clear()
        .process_group();
    for key in ["PATH", "HOME"] {
        if let Some(v) = value(key) {
            command = command.env(key, v);
        }
    }
    let name = tool.as_str();
    let exit = |code: Option<i32>| {
        let said = proc::last_log_line(&log).unwrap_or_else(|| "no output".to_owned());
        let code = code.map_or_else(|| "a signal".to_owned(), |c| c.to_string());
        ToolCheck::Broken(format!("`{name} --version` ended with {code}: {said}"))
    };
    match proc::to_log_with_deadline(&command, &log, PROBE_DEADLINE, &|_| false) {
        Ok(Finished::Exited(Some(0))) => ToolCheck::Ready,
        Ok(Finished::Exited(code)) => exit(code),
        Ok(Finished::TimedOut | Finished::Stopped) => ToolCheck::Broken(format!(
            "`{name} --version` did not answer in {}s",
            PROBE_DEADLINE.as_secs()
        )),
        Err(error) => ToolCheck::Broken(error.to_string()),
    }
}

/// `Ok` when the run may start. Only a `runtime: host` agent with a `tool` is
/// checked; a host that cannot be described (no `HOME`) is left to the run,
/// which reports it.
pub(super) fn check(
    spec: &AgentSpec,
    host: &dyn Fn() -> Result<HostDeps, Failure>,
    scratch: &Utf8Path,
) -> Result<(), Block> {
    let Launch::Tool { tool, .. } = &spec.launch else {
        return Ok(());
    };
    if spec.runtime != Runtime::Host {
        return Ok(());
    }
    let Ok(deps) = host() else {
        return Ok(());
    };
    match (deps.tool_check)(*tool, &deps.parent_env, scratch) {
        ToolCheck::Ready => {}
        ToolCheck::Missing => {
            return Err(Block {
                code: UNAVAILABLE,
                reason: "provider unavailable",
                detail: format!(
                    "`{tool}` is not installed or not on PATH",
                    tool = tool.as_str()
                ),
            });
        }
        ToolCheck::Broken(why) => {
            return Err(Block {
                code: UNAVAILABLE,
                reason: "provider unavailable",
                detail: format!("`{tool}` does not run: {why}", tool = tool.as_str()),
            });
        }
    }
    logged_in(spec, *tool, &deps)
}

/// A key the agent lists in `secrets` logs the tool in; so does a login file
/// that holds a login. Whether the login is still honoured is the tool's to
/// say, and shows in the run.
fn logged_in(spec: &AgentSpec, tool: Tool, deps: &HostDeps) -> Result<(), Block> {
    if tool
        .api_key_vars()
        .iter()
        .any(|key| spec.secrets.iter().any(|s| s == key))
    {
        return Ok(());
    }
    let var = |key: &str| {
        deps.parent_env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let mut broken = Vec::new();
    let mut absent = Vec::new();
    for login in tool.login_files(&deps.home, &var) {
        match fs::read_bytes(&login.real) {
            Ok(Some(bytes)) if is_login(&bytes) => return Ok(()),
            Ok(Some(_)) => broken.push(format!("`{}`", login.real)),
            Ok(None) => absent.push(format!("`{}`", login.real)),
            Err(error) => broken.push(format!("`{}` ({})", login.real, Failure::from(error).what)),
        }
    }
    let name = tool.as_str();
    let detail = if broken.is_empty() {
        format!(
            "`{name}` is not logged in: no login at {}. Log in once interactively, or list an API key ({}) in `secrets`",
            absent.join(", "),
            tool.api_key_vars().join(", ")
        )
    } else {
        format!(
            "`{name}`'s login {} is unreadable, empty or not valid JSON. Log in again",
            broken.join(", ")
        )
    };
    Err(Block {
        code: AUTH,
        reason: "auth error",
        detail,
    })
}
