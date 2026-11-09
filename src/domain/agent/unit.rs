//! The systemd user units ivar writes for a scheduled agent or an Agent Task,
//! and how it recognises its own units again.

use super::spec::{AgentSpec, Timing};

/// The `[Unit]` key that marks a unit as written by a given hall.
pub const OWNED_KEY: &str = "X-Ivar-Owned";

/// `ivar-agent-<hall_hash>-<agent>`, without a suffix.
#[must_use]
pub fn unit_stem(hall_hash: &str, agent: &str) -> String {
    format!("{}{agent}", unit_prefix(hall_hash))
}

/// The stem prefix every unit of this hall shares.
#[must_use]
pub fn unit_prefix(hall_hash: &str) -> String {
    format!("ivar-agent-{hall_hash}-")
}

/// The oneshot service a timer fires.
///
/// `TimeoutStartSec=infinity` because a oneshot's default start timeout would
/// kill a long run; ivar's own watchdog enforces the agent timeout.
///
/// `path` is pinned because a `Persistent=true` catch-up fires as the user
/// manager starts, before the login session has imported its `PATH`, so a
/// tool in `~/.local/bin` would not be found.
#[must_use]
pub fn render_service(
    hall_hash: &str,
    agent: &str,
    hall_root: &str,
    ivar_bin: &str,
    path: Option<&str>,
) -> String {
    let environment = path
        .map(|p| format!("Environment=\"PATH={}\"\n", escape_quoted(p)))
        .unwrap_or_default();
    format!(
        "[Unit]\nDescription=ivar agent {agent} ({hall_root})\n{OWNED_KEY}={hall_hash}\n\n\
         [Service]\nType=oneshot\nTimeoutStartSec=infinity\nWorkingDirectory={hall_root}\n\
         {environment}ExecStart={ivar_bin} agent run {agent} --trigger schedule\n"
    )
}

/// The `PATH` a service pins, unescaped.
#[must_use]
pub fn service_path(service_text: &str) -> Option<String> {
    service_text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Environment=\"PATH="))
        .and_then(|rest| rest.strip_suffix('"'))
        .map(unescape_quoted)
}

/// systemd expands `%` specifiers and reads C escapes inside a quoted value.
fn escape_quoted(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
}

fn unescape_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '%' => {
                out.push('%');
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// The timer that fires the agent's service on `on_calendar`.
///
/// With `catchup`, `Persistent=true` makes systemd run the agent once when the
/// timer comes back after a missed elapse, however many were missed.
#[must_use]
pub fn render_timer(hall_hash: &str, agent: &str, on_calendar: &str, catchup: bool) -> String {
    let stem = unit_stem(hall_hash, agent);
    let persistent = if catchup { "Persistent=true\n" } else { "" };
    format!(
        "[Unit]\nDescription=ivar agent {agent} schedule\n{OWNED_KEY}={hall_hash}\n\n\
         [Timer]\nOnCalendar={on_calendar}\n{persistent}Unit={stem}.service\n\n\
         [Install]\nWantedBy=timers.target\n"
    )
}

/// The one-shot timer of an Agent Task: it elapses once, at `on_calendar`.
///
/// `Persistent=true` always, so a machine that was off or asleep at that time
/// runs the task when it comes back instead of dropping it.
#[must_use]
pub fn render_once_timer(hall_hash: &str, agent: &str, on_calendar: &str) -> String {
    let stem = unit_stem(hall_hash, agent);
    format!(
        "[Unit]\nDescription=ivar agent {agent} one-shot task\n{OWNED_KEY}={hall_hash}\n\n\
         [Timer]\nOnCalendar={on_calendar}\nPersistent=true\nUnit={stem}.service\n\n\
         [Install]\nWantedBy=timers.target\n"
    )
}

/// The timer a spec needs: one-shot for an Agent Task, recurring otherwise.
#[must_use]
pub fn render_spec_timer(hall_hash: &str, spec: &AgentSpec) -> String {
    match &spec.timing {
        Timing::Once(at) => render_once_timer(hall_hash, &spec.name, &at.on_calendar()),
        Timing::Recurring(schedule) => {
            render_timer(hall_hash, &spec.name, schedule.on_calendar(), spec.catchup)
        }
    }
}

/// Whether `unit_text` carries this hall's ownership marker on a line of its own.
#[must_use]
pub fn owned_by(unit_text: &str, hall_hash: &str) -> bool {
    let marker = format!("{OWNED_KEY}={hall_hash}");
    unit_text.lines().any(|line| line.trim() == marker)
}

/// The program a service's `ExecStart=` runs.
#[must_use]
pub fn exec_binary(service_text: &str) -> Option<&str> {
    service_text
        .lines()
        .find_map(|line| line.trim().strip_prefix("ExecStart="))
        .and_then(|command| command.split_whitespace().next())
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/unit.rs"]
mod tests;
