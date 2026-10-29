//! `ivar agent status`: each agent's schedule, next and last run, and unit drift.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use super::hall_hash;
use super::sync::current_ivar_bin;
use crate::action::{Ctx, discover_hall};
use crate::domain::agent::receipt::AgentRunOutcome;
use crate::domain::agent::unit::{exec_binary, render_service, render_timer, unit_stem};
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::{fs, systemd, table};
use crate::store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum UnitSync {
    InSync,
    NotInstalled,
    Stale,
    BinaryMissing { path: String },
    Invalid { reason: String },
}

impl UnitSync {
    fn label(&self) -> &'static str {
        match self {
            Self::InSync => "in sync",
            Self::NotInstalled => "not installed",
            Self::Stale => "stale",
            Self::BinaryMissing { .. } => "binary missing",
            Self::Invalid { .. } => "invalid",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub name: String,
    pub schedule: Option<String>,
    pub on_calendar: Option<String>,
    pub next_run: Option<String>,
    pub last_run_id: Option<String>,
    pub last_outcome: Option<AgentRunOutcome>,
    pub sync: UnitSync,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub root: Utf8PathBuf,
    pub agents: Vec<AgentStatus>,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.agents.is_empty() {
            return writeln!(w, "No agents in {}.", self.root);
        }
        let mut t = table::new(&["NAME", "SCHEDULE", "NEXT", "LAST", "SYNC"]);
        for agent in &self.agents {
            let last = agent
                .last_outcome
                .as_ref()
                .and_then(|o| serde_json::to_value(o).ok())
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "-".to_owned());
            t.add_row(vec![
                agent.name.as_str(),
                agent.schedule.as_deref().unwrap_or("-"),
                agent.next_run.as_deref().unwrap_or("-"),
                &last,
                agent.sync.label(),
            ]);
        }
        table::write(w, &t)
    }
}

fn unit_sync(
    installed_service: Option<&str>,
    installed_timer: Option<&str>,
    want_service: &str,
    want_timer: &str,
) -> UnitSync {
    match (installed_service, installed_timer) {
        (None, _) | (_, None) => UnitSync::NotInstalled,
        (Some(s), Some(_)) if exec_binary(s).is_some_and(|b| !Utf8Path::new(b).exists()) => {
            UnitSync::BinaryMissing {
                path: exec_binary(s).unwrap_or_default().to_owned(),
            }
        }
        (Some(s), Some(t)) if s == want_service && t == want_timer => UnitSync::InSync,
        _ => UnitSync::Stale,
    }
}

/// # Errors
///
/// See [`status_in`]; also when the unit directory or the running binary is unknown.
pub fn status(ctx: &Ctx) -> Outcome<StatusOutcome> {
    status_in(ctx, &systemd::user_unit_dir()?, &current_ivar_bin()?)
}

/// # Errors
///
/// When there is no hall or the agent directory or a unit file cannot be read;
/// an invalid agent file is reported in its row, never as a failure.
pub fn status_in(ctx: &Ctx, unit_dir: &Utf8Path, ivar_bin: &Utf8Path) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let hash = hall_hash(&layout);
    let mut agents = Vec::new();
    for file in store::agent::list_files(&layout)? {
        let last = store::agent::latest_receipt(&layout, &file.name)?;
        let mut status = AgentStatus {
            name: file.name.clone(),
            schedule: None,
            on_calendar: None,
            next_run: None,
            last_run_id: last.as_ref().map(|r| r.run_id.clone()),
            last_outcome: last.map(|r| r.outcome),
            sync: UnitSync::NotInstalled,
        };
        match store::agent::parse(&file) {
            Err(failure) => {
                status.sync = UnitSync::Invalid {
                    reason: failure.what,
                }
            }
            Ok(spec) => {
                let stem = unit_stem(&hash, &spec.name);
                let service = fs::read_text(&unit_dir.join(format!("{stem}.service")))?;
                let timer = fs::read_text(&unit_dir.join(format!("{stem}.timer")))?;
                status.sync = unit_sync(
                    service.as_deref(),
                    timer.as_deref(),
                    &render_service(&hash, &spec.name, layout.root().as_str(), ivar_bin.as_str()),
                    &render_timer(&hash, &spec.name, spec.schedule.on_calendar(), spec.catchup),
                );
                if matches!(status.sync, UnitSync::InSync | UnitSync::Stale) {
                    status.next_run = systemd::next_elapse(&format!("{stem}.timer"));
                }
                status.schedule = Some(spec.schedule.as_str().to_owned());
                status.on_calendar = Some(spec.schedule.on_calendar().to_owned());
            }
        }
        agents.push(status);
    }
    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        agents,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/status.rs"]
mod tests;
