//! The systemd user units ivar writes for a scheduled agent, and how it
//! recognises its own units again.

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
#[must_use]
pub fn render_service(hall_hash: &str, agent: &str, hall_root: &str, ivar_bin: &str) -> String {
    format!(
        "[Unit]\nDescription=ivar agent {agent} ({hall_root})\n{OWNED_KEY}={hall_hash}\n\n\
         [Service]\nType=oneshot\nTimeoutStartSec=infinity\nWorkingDirectory={hall_root}\n\
         ExecStart={ivar_bin} agent run {agent} --trigger schedule\n"
    )
}

/// The timer that fires the agent's service on `on_calendar`.
#[must_use]
pub fn render_timer(hall_hash: &str, agent: &str, on_calendar: &str) -> String {
    let stem = unit_stem(hall_hash, agent);
    format!(
        "[Unit]\nDescription=ivar agent {agent} schedule\n{OWNED_KEY}={hall_hash}\n\n\
         [Timer]\nOnCalendar={on_calendar}\nUnit={stem}.service\n\n\
         [Install]\nWantedBy=timers.target\n"
    )
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
