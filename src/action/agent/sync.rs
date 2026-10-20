//! `ivar agent sync`: reconcile systemd user timers with `agents/*.md`.

use std::collections::BTreeSet;
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use super::hall_hash;
use super::requirements::{self, Probes};
use crate::action::{Ctx, discover_hall};
use crate::domain::agent::unit::{owned_by, render_service, render_timer, unit_prefix, unit_stem};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::{fs, systemd};
use crate::store;

#[derive(Debug)]
pub struct SyncEnv<'a> {
    pub unit_dir: Utf8PathBuf,
    pub ivar_bin: Utf8PathBuf,
    pub is_linux: bool,
    pub probes: Probes<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    pub root: Utf8PathBuf,
    pub unit_dir: Utf8PathBuf,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

impl WriteHuman for SyncOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let lists = [
            ("created", &self.created),
            ("updated", &self.updated),
            ("removed", &self.removed),
            ("unchanged", &self.unchanged),
        ];
        if lists.iter().all(|(_, names)| names.is_empty()) {
            return writeln!(w, "No agents.");
        }
        for (label, names) in lists {
            if !names.is_empty() {
                writeln!(w, "{label}: {}", names.join(", "))?;
            }
        }
        Ok(())
    }
}

/// # Errors
///
/// See [`sync_with`]; also `agent.ivar_path_unknown` when the running binary has no path.
pub fn sync(ctx: &Ctx) -> Outcome<SyncOutcome> {
    let ivar_bin = current_ivar_bin()?;
    sync_with(
        ctx,
        &SyncEnv {
            unit_dir: systemd::user_unit_dir()?,
            ivar_bin,
            is_linux: cfg!(target_os = "linux"),
            probes: requirements::system_probes(),
        },
    )
}

pub(crate) fn current_ivar_bin() -> Result<Utf8PathBuf, Failure> {
    std::env::current_exe()
        .ok()
        .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
        .ok_or_else(|| {
            Failure::blocked(
                "agent.ivar_path_unknown",
                "cannot resolve the path of the running ivar binary",
            )
        })
}

/// # Errors
///
/// `agent.unsupported_platform` off Linux, `agent.invalid` when any file fails
/// validation (nothing is written then), `agent.unit_not_owned` when a unit of
/// the same name belongs to someone else, or a filesystem/systemctl failure.
pub fn sync_with(ctx: &Ctx, env: &SyncEnv<'_>) -> Outcome<SyncOutcome> {
    if !env.is_linux {
        return Err(Failure::blocked(
            "agent.unsupported_platform",
            "`ivar agent sync` needs Linux with systemd; agent timers are not supported on this system",
        ));
    }
    let layout = discover_hall(ctx)?;
    let specs = store::agent::list_files(&layout)?
        .iter()
        .map(store::agent::parse)
        .collect::<Result<Vec<_>, _>>()?;
    let hash = hall_hash(&layout);
    fs::ensure_dir(&env.unit_dir)?;
    let mut outcome = SyncOutcome {
        root: layout.root().to_path_buf(),
        unit_dir: env.unit_dir.clone(),
        created: vec![],
        updated: vec![],
        removed: vec![],
        unchanged: vec![],
    };
    let mut wanted = BTreeSet::new();
    for spec in &specs {
        let stem = unit_stem(&hash, &spec.name);
        wanted.insert(stem.clone());
        let service_path = env.unit_dir.join(format!("{stem}.service"));
        let timer_path = env.unit_dir.join(format!("{stem}.timer"));
        let service = render_service(
            &hash,
            &spec.name,
            layout.root().as_str(),
            env.ivar_bin.as_str(),
        );
        let timer = render_timer(&hash, &spec.name, spec.schedule.on_calendar());
        let old_service = fs::read_text(&service_path)?;
        let old_timer = fs::read_text(&timer_path)?;
        let bucket = match (&old_service, &old_timer) {
            (Some(s), Some(t)) if *s == service && *t == timer => {
                outcome.unchanged.push(spec.name.clone());
                continue;
            }
            (None, None) => &mut outcome.created,
            (s, t) if s.iter().chain(t.iter()).any(|text| !owned_by(text, &hash)) => {
                return Err(Failure::blocked(
                    "agent.unit_not_owned",
                    format!(
                        "{service_path} exists but is not owned by this hall; rename the agent or remove the unit"
                    ),
                ));
            }
            _ => &mut outcome.updated,
        };
        bucket.push(spec.name.clone());
        fs::write_atomic(&service_path, service.as_bytes())?;
        fs::write_atomic(&timer_path, timer.as_bytes())?;
    }
    let prefix = unit_prefix(&hash);
    for path in fs::read_dir(&env.unit_dir)? {
        let Some(stem) = path.file_name().and_then(|n| n.strip_suffix(".timer")) else {
            continue;
        };
        if !stem.starts_with(&prefix) || wanted.contains(stem) {
            continue;
        }
        if !fs::read_text(&path)?.is_some_and(|text| owned_by(&text, &hash)) {
            continue;
        }
        systemd::disable_now(&format!("{stem}.timer"))?;
        fs::remove_path(&path)?;
        fs::remove_path(&env.unit_dir.join(format!("{stem}.service")))?;
        outcome
            .removed
            .push(stem.trim_start_matches(&prefix).to_owned());
    }
    if !(outcome.created.is_empty() && outcome.updated.is_empty() && outcome.removed.is_empty()) {
        systemd::daemon_reload()?;
    }
    for name in outcome.created.iter().chain(&outcome.updated) {
        systemd::enable_now(&format!("{}.timer", unit_stem(&hash, name)))?;
    }
    outcome.removed.sort();
    let warnings = requirements::as_warnings(&requirements::diagnose_agents(
        !specs.is_empty(),
        &env.probes,
    ));
    Ok(Report::with_warnings(outcome, warnings))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/sync.rs"]
mod tests;
