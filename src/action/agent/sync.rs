//! `ivar agent sync`: reconcile systemd user timers with `agents/*.md`.

use std::collections::BTreeSet;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use super::hall_hash;
use super::requirements::{self, Probes};
use crate::action::{Ctx, discover_hall};
use crate::domain::agent::spec::{AgentSpec, Runtime, WritePath};
use crate::domain::agent::unit::{
    owned_by, render_service, render_spec_timer, unit_prefix, unit_stem,
};
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::infra::{clock, fs, systemd};
use crate::store;
use crate::store::layout::Layout;

/// What time it is and how this machine reads a time without an offset,
/// injectable so tests never depend on the clock or the time zone.
pub struct Clock<'a> {
    pub now_secs: i64,
    pub local_epoch_secs: &'a dyn Fn(&str) -> Option<i64>,
}

impl Clock<'static> {
    /// This machine's clock.
    #[must_use]
    pub fn system() -> Self {
        Self {
            now_secs: clock::now_epoch_secs(),
            local_epoch_secs: &clock::local_epoch_secs,
        }
    }
}

impl std::fmt::Debug for Clock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clock")
            .field("now_secs", &self.now_secs)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct SyncEnv<'a> {
    pub unit_dir: Utf8PathBuf,
    pub ivar_bin: Utf8PathBuf,
    /// The `PATH` each service pins; see [`render_service`].
    pub path: Option<String>,
    pub is_linux: bool,
    pub probes: Probes<'a>,
    pub clock: Clock<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    pub root: Utf8PathBuf,
    pub unit_dir: Utf8PathBuf,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    /// Agent Tasks whose `at` had already passed, started now as a timer run.
    pub fired: Vec<String>,
}

impl WriteHuman for SyncOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let lists = [
            ("created", &self.created),
            ("updated", &self.updated),
            ("removed", &self.removed),
            ("unchanged", &self.unchanged),
        ];
        if lists.iter().all(|(_, names)| names.is_empty()) && self.fired.is_empty() {
            return writeln!(w, "No agents.");
        }
        for (label, names) in lists {
            if !names.is_empty() {
                writeln!(w, "{label}: {}", names.join(", "))?;
            }
        }
        if !self.fired.is_empty() {
            writeln!(
                w,
                "fired now (`at` already passed, started as a timer run): {}",
                self.fired.join(", ")
            )?;
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
            path: std::env::var("PATH").ok().filter(|p| !p.is_empty()),
            is_linux: cfg!(target_os = "linux"),
            probes: requirements::system_probes(),
            clock: Clock::system(),
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
        fired: vec![],
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
            env.path.as_deref(),
        );
        let timer = render_spec_timer(&hash, spec);
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
    outcome.fired = fire_due_tasks(&layout, &specs, &hash, &env.clock)?;
    let mut warnings =
        requirements::as_warnings(&requirements::diagnose_agents(&specs, &env.probes));
    warnings.extend(specs.iter().filter(|s| s.runtime == Runtime::Host).map(|s| {
        Warning::new(
            "agent.host_ambient_read",
            "agents",
            format!(
                "`{}` runs on the host: git gets no credential helper or credential variables, but it can read everything your user can (SSH keys, ~/.config/gh), reach sockets in /run/user/<uid>, and its network is not filtered, so it could push with a readable SSH key.",
                s.name
            ),
        )
    }));
    warnings.extend(proc_write_warnings(&specs));
    warnings.extend(build_dir_warning(&env.ivar_bin, specs.len()));
    Ok(Report::with_warnings(outcome, warnings))
}

/// Starts every Agent Task whose `at` is not in the future: its timer has no
/// elapse left to wait for, and a persistent timer only catches up on a stamp
/// it already has.
///
/// A task whose run is still going (it holds the agent lock) is left alone:
/// starting it again would only record a `skipped` run and notify about it.
fn fire_due_tasks(
    layout: &Layout,
    specs: &[AgentSpec],
    hash: &str,
    clock: &Clock<'_>,
) -> Result<Vec<String>, Failure> {
    let mut fired = Vec::new();
    for spec in specs {
        let due = spec.at().is_some_and(|at| {
            at.epoch_secs(clock.local_epoch_secs)
                .is_some_and(|secs| secs <= clock.now_secs)
        });
        if due && !is_running(layout, &spec.name)? {
            systemd::start_no_block(&format!("{}.service", unit_stem(hash, &spec.name)))?;
            fired.push(spec.name.clone());
        }
    }
    Ok(fired)
}

fn is_running(layout: &Layout, name: &str) -> Result<bool, Failure> {
    Ok(fs::try_lock_exclusive(&layout.agent_lock(name))?.is_none())
}

fn build_dir_warning(ivar_bin: &Utf8Path, agents: usize) -> Option<Warning> {
    let in_build_dir = ivar_bin
        .ancestors()
        .filter_map(Utf8Path::parent)
        .any(|dir| dir.file_name() == Some("target"));
    (in_build_dir && agents > 0).then(|| {
        Warning::new(
            "agent.ivar_bin_in_build_dir",
            "agents",
            format!(
                "the timers run `{ivar_bin}`, a cargo build output that `cargo clean` or a rebuild removes or changes; run `ivar agent sync` from the installed `ivar` instead"
            ),
        )
    })
}

fn proc_write_warnings(specs: &[AgentSpec]) -> Vec<Warning> {
    specs
        .iter()
        .filter(|s| s.write.iter().any(writes_proc))
        .map(|s| {
            Warning::new(
                "agent.host_write_proc",
                "agents",
                format!(
                    "`{}` can write /proc. Landlock cannot narrow that to the run's own processes, so it can also change the /proc entries of your other processes that the kernel lets your user write (such as comm or oom_score_adj); their memory stays behind ptrace rules.",
                    s.name
                ),
            )
        })
        .collect()
}

fn writes_proc(write: &WritePath) -> bool {
    matches!(write, WritePath::Absolute(path) if path.starts_with("/proc"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/sync.rs"]
mod tests;
