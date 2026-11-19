//! `ivar agent run`: one sandboxed run of an agent, always ending in a receipt.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use uuid::Uuid;

use super::conclude;
use super::feature_access::{self, Delivered};
use super::notify;
use super::record;
use super::scripts;
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::agent::container::{
    ContainerPlan, Mount, agent_run_args, proxy_run_args, tinyproxy_conf, tinyproxy_filter,
};
use crate::domain::agent::current::CurrentRun;
use crate::domain::agent::images::{BASE_IMAGE, PROXY_IMAGE};
use crate::domain::agent::receipt::{
    self, AgentRunReceipt, Trigger, classify, declared_outcome, scrub,
};
use crate::domain::agent::spec::{Access, AgentSpec, Launch, Runtime};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, Status, Warning, WriteHuman};
use crate::infra::proc::{self, Finished};
use crate::infra::{docker, fs};
use crate::store;
use crate::store::agent_secrets::{AgentSecrets, render_env_file};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

mod host;
mod network;
mod preflight;

pub(crate) use host::HostDeps;
use host::execute_host;
#[cfg(test)]
use host::{host_workspace, seed_credentials, write_back_credentials, write_roots};
pub(crate) use network::resolves;
#[cfg(test)]
pub(crate) use preflight::ToolCheck;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInput {
    pub name: String,
    pub trigger: Trigger,
}

impl RunInput {
    /// # Errors
    ///
    /// `agent.invalid_trigger` naming the accepted triggers.
    pub fn from_cli(name: String, trigger: &str) -> Result<Self, Failure> {
        let trigger = trigger
            .parse::<Trigger>()
            .map_err(|e| Failure::failed("agent.invalid_trigger", e))?;
        Ok(Self { name, trigger })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Workspace {
    pub mounts: Vec<Mount>,
    pub workdir: String,
}

pub(super) struct Ran {
    pub(super) finished: Finished,
    pub(super) output: String,
}

struct Attempt {
    ran: Ran,
    delivered: Option<Delivered>,
}

/// # Errors
///
/// A [`Failure`] only when the hall or the agent file cannot be loaded, or
/// the receipt cannot be written; every other failure is a `failed` receipt.
pub fn run(ctx: &Ctx, input: RunInput) -> Outcome<AgentRunReceipt> {
    run_with(ctx, input, &HostDeps::system, &network::resolves)
}

pub(crate) fn run_with(
    ctx: &Ctx,
    input: RunInput,
    host: &dyn Fn() -> Result<HostDeps, Failure>,
    resolves: &dyn Fn(&str) -> bool,
) -> Outcome<AgentRunReceipt> {
    let RunInput { name, trigger } = input;
    let layout = discover_hall(ctx)?;
    let spec = store::agent::load(&layout, &name)?;
    run_spec(ctx, &layout, &spec, trigger, host, resolves)
}

/// One run of `spec`, which need not be a file in the hall's `agents/`: the
/// agents `ivar` ships inside the binary run through here too, with the same
/// lock, sandbox and receipt as any other.
///
/// This is the whole run and nothing more. Whatever a caller does *after* an
/// agent ran, such as syncing the hall, belongs to the caller. An agent that
/// `ivar` runs while it is itself syncing the hall must come through here and
/// not through [`run`], so that its run cannot start another sync.
pub(crate) fn run_spec(
    ctx: &Ctx,
    layout: &Layout,
    spec: &AgentSpec,
    trigger: Trigger,
    host: &dyn Fn() -> Result<HostDeps, Failure>,
    resolves: &dyn Fn(&str) -> bool,
) -> Outcome<AgentRunReceipt> {
    let started_at = rfc3339_now();
    let run_id = receipt::run_id(&started_at, &Uuid::new_v4().simple().to_string()[..8]);
    let lock = match fs::try_lock_exclusive(&layout.agent_lock(&spec.name)) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            let skipped = AgentRunReceipt::skipped(run_id, spec.name.clone(), trigger, started_at);
            return finish(layout, spec, skipped);
        }
        Err(error) => {
            let failure = Failure::from(error);
            let receipt = failure_receipt(run_id, spec, trigger, started_at, &failure, &[]);
            return finish(layout, spec, receipt);
        }
    };
    // Dropped before `lock` on every path: the record is gone before the lock is let go.
    let running = RunningMark::set(layout, &spec.name, &run_id, trigger, &started_at);
    // Before the DNS wait and before anything is launched or mounted.
    if let Err(failure) = scripts::ensure_approved(layout, spec) {
        let receipt = failure_receipt(run_id, spec, trigger, started_at, &failure, &[]);
        let result = finish(layout, spec, receipt);
        drop((running, lock));
        return result;
    }
    let scratch = layout.agent_run_scratch(&run_id);
    let checked = preflight::check(spec, host, &scratch);
    let _ = fs::remove_path(&scratch);
    if let Err(block) = checked {
        let blocked = AgentRunReceipt::blocked(
            run_id,
            spec.name.clone(),
            trigger,
            started_at,
            rfc3339_now(),
            block.message(&spec.name),
        );
        let result = finish(layout, spec, blocked);
        drop((running, lock));
        return result;
    }
    if trigger == Trigger::Schedule
        && let Launch::Tool { tool, .. } = spec.launch
        && let Some(api) = tool.api_host()
    {
        network::wait_for_dns(
            api,
            resolves,
            &std::thread::sleep,
            network::BUDGET,
            network::INTERVAL,
        );
    }
    let mut secret_values: Vec<String> = Vec::new();
    let attempt = (|| -> Result<Attempt, Failure> {
        let secrets = AgentSecrets::read(layout)?.select(&spec.secrets)?;
        secret_values = secrets.iter().map(|(_, v)| v.clone()).collect();
        let (ws, worktree) = workspace_for(ctx, layout, spec)?;
        let ran = match spec.runtime {
            Runtime::Docker => execute(spec, &run_id, &scratch, &secrets, ws)?,
            Runtime::Host => {
                let state = layout.agent_tool_state(&spec.name, tool_name(spec));
                execute_host(
                    spec,
                    layout.root(),
                    &scratch,
                    &state,
                    &secrets,
                    &ws,
                    &host()?,
                )?
            }
        };
        let delivered =
            worktree.map(|wt| feature_access::commit_and_preview(ctx, layout, spec, &wt, &run_id));
        Ok(Attempt { ran, delivered })
    })();
    let _ = fs::remove_path(&scratch);
    let values: Vec<&str> = secret_values.iter().map(String::as_str).collect();
    let receipt = match attempt {
        Err(failure) => failure_receipt(run_id, spec, trigger, started_at, &failure, &values),
        Ok(attempt) => completed(spec, run_id, trigger, started_at, &attempt, &values),
    };
    let result = finish(layout, spec, receipt);
    drop(running);
    drop(lock);
    result
}

/// The receipt of a run that stopped on `failure`: `blocked` when `ivar`
/// refused a precondition before doing anything (`Status::Blocked`, which a
/// person can fix and `ivar agent resume` then retries), `failed` otherwise.
fn failure_receipt(
    run_id: String,
    spec: &AgentSpec,
    trigger: Trigger,
    started_at: String,
    failure: &Failure,
    values: &[&str],
) -> AgentRunReceipt {
    let make = match failure.status {
        Status::Blocked => AgentRunReceipt::blocked,
        Status::Failed => AgentRunReceipt::failed,
    };
    make(
        run_id,
        spec.name.clone(),
        trigger,
        started_at,
        rfc3339_now(),
        scrub(&failure.what, values),
    )
}

/// The record of the run in progress, from taking the agent's lock until the
/// run is over. Best effort: a status that cannot tell a run is going is a
/// lesser harm than a run that does not happen.
struct RunningMark<'a> {
    layout: &'a Layout,
    agent: String,
    run_id: String,
}

impl<'a> RunningMark<'a> {
    fn set(
        layout: &'a Layout,
        agent: &str,
        run_id: &str,
        trigger: Trigger,
        started_at: &str,
    ) -> Self {
        let pid = std::process::id();
        let current = CurrentRun {
            run_id: run_id.to_owned(),
            trigger,
            started_at: started_at.to_owned(),
            pid,
            pid_start: proc::start_time(pid),
        };
        let _ = store::agent::write_current_run(layout, agent, &current);
        Self {
            layout,
            agent: agent.to_owned(),
            run_id: run_id.to_owned(),
        }
    }
}

impl Drop for RunningMark<'_> {
    fn drop(&mut self) {
        let _ = store::agent::remove_current_run(self.layout, &self.agent, &self.run_id);
    }
}

fn tool_name(spec: &AgentSpec) -> &'static str {
    match spec.launch {
        Launch::Tool { tool, .. } => tool.as_str(),
        Launch::Image { .. } => "image",
    }
}

fn completed(
    spec: &AgentSpec,
    run_id: String,
    trigger: Trigger,
    started_at: String,
    attempt: &Attempt,
    values: &[&str],
) -> AgentRunReceipt {
    let raw_output = scrub(&attempt.ran.output, values);
    let tool = match spec.launch {
        Launch::Tool { tool, .. } => Some(tool),
        Launch::Image { .. } => None,
    };
    let output = match tool {
        Some(t) => t.final_message(&raw_output),
        None => raw_output.clone(),
    };
    let blocked = attempt.ran.finished == Finished::Stopped
        || tool.is_some_and(|t| t.is_blocked(&raw_output));
    let declared = tool.and_then(|_| declared_outcome(&output));
    let (exit_code, timed_out) = match attempt.ran.finished {
        Finished::Exited(code) => (code, false),
        Finished::TimedOut => (None, true),
        Finished::Stopped => (None, false),
    };
    let mut receipt = AgentRunReceipt {
        outcome: classify(exit_code, timed_out, blocked, declared),
        exit_code,
        cost_usd: tool.and_then(|t| t.cost_usd(&raw_output)),
        output,
        ended_at: rfc3339_now(),
        ..AgentRunReceipt::skipped(run_id, spec.name.clone(), trigger, started_at)
    };
    if let Some(d) = &attempt.delivered {
        receipt.diff = d.diff.as_deref().map(|t| scrub(t, values));
        receipt.delivery_fingerprint.clone_from(&d.fingerprint);
        receipt.delivery_error = d.error.as_deref().map(|t| scrub(t, values));
    }
    receipt
}

/// The single point that records a run, and concludes an Agent Task's.
pub(crate) fn finish(
    layout: &Layout,
    spec: &AgentSpec,
    mut receipt: AgentRunReceipt,
) -> Outcome<AgentRunReceipt> {
    receipt.scheduled_at = spec.at().map(|at| at.to_string());
    notify::deliver(layout, spec, &mut receipt);
    notify::problem(layout, spec, &mut receipt);
    store::agent::write_receipt(layout, &receipt)?;
    let mut report = Report::new(receipt);
    if conclude::concludes(spec, &report.value) {
        match conclude::move_to_done(layout, spec, &report.value.run_id) {
            Ok(_) => conclude::report_concluded(layout, spec, &report.value),
            Err(failure) => report.warn(Warning::new(
                "agent.task_not_concluded",
                &spec.name,
                failure.what,
            )),
        }
    }
    // After the task file has moved, so the move is swept in the same sync.
    // The receipt is already on disk; it is only rewritten to add what the
    // sync could not do, and a failure to do so is not worth failing the run.
    if record::records(&report.value) {
        let warnings = record::record_run(layout, spec, &report.value);
        if !warnings.is_empty() {
            report.value.sync_warnings = warnings;
            let _ = store::agent::write_receipt(layout, &report.value);
        }
    }
    Ok(report)
}

fn workspace_for(
    ctx: &Ctx,
    layout: &Layout,
    spec: &AgentSpec,
) -> Result<(Workspace, Option<Utf8PathBuf>), Failure> {
    match spec.access {
        Access::Read => Ok((read_workspace(layout, &read_manifest(layout)?, spec), None)),
        Access::Feature => {
            feature_access::prepare(ctx, layout, spec).map(|(ws, wt)| (ws, Some(wt)))
        }
    }
}

/// Worktree content only: the `.bare` clone, with its config and remotes, is never mounted.
pub(crate) fn read_workspace(layout: &Layout, manifest: &Manifest, spec: &AgentSpec) -> Workspace {
    let mounts = manifest
        .repos()
        .iter()
        .map(|repo| Mount {
            source: layout
                .repo_worktree(repo.name(), repo.default_branch())
                .to_string(),
            target: format!("/workspace/{}", repo.name()),
            read_only: true,
        })
        .collect();
    let workdir = spec
        .workdir
        .as_deref()
        .map_or_else(|| "/workspace".to_owned(), |w| format!("/workspace/{w}"));
    Workspace { mounts, workdir }
}

fn execute(
    spec: &AgentSpec,
    run_id: &str,
    scratch: &Utf8Path,
    secrets: &[(String, String)],
    ws: Workspace,
) -> Result<Ran, Failure> {
    let net = format!("ivar-agent-{run_id}-net");
    let agent = format!("ivar-agent-{run_id}");
    let proxy = format!("ivar-agent-{run_id}-proxy");
    let proxy_dir = scratch.join("proxy");
    fs::write_sensitive_atomic(
        &scratch.join("secrets.env"),
        render_env_file(secrets)?.as_bytes(),
    )?;
    fs::ensure_dir(&proxy_dir)?;
    fs::write_atomic(
        &proxy_dir.join("tinyproxy.conf"),
        tinyproxy_conf().as_bytes(),
    )?;
    fs::write_atomic(
        &proxy_dir.join("filter"),
        tinyproxy_filter(&spec.network).as_bytes(),
    )?;
    docker::network_create_internal(&net)?;
    let ran = (|| -> Result<Ran, Failure> {
        docker::run_detached(&proxy_run_args(
            &proxy,
            &net,
            proxy_dir.as_str(),
            PROXY_IMAGE,
        ))?;
        docker::network_connect("bridge", &proxy)?;
        let (image, argv, env, tool) = match &spec.launch {
            Launch::Tool { tool, model } => (
                BASE_IMAGE.to_owned(),
                tool.argv(model.as_deref(), &spec.prompt),
                tool.env(),
                Some(*tool),
            ),
            Launch::Image { image, command } => (image.clone(), command.clone(), Vec::new(), None),
        };
        let plan = ContainerPlan {
            name: agent.clone(),
            network: net.clone(),
            image,
            env,
            env_file: scratch.join("secrets.env").to_string(),
            mounts: ws.mounts,
            workdir: ws.workdir,
            argv,
        };
        let log = scratch.join("output.log");
        let asked = |seen: &str| tool.is_some_and(|t| t.is_blocked(seen));
        let finished = proc::to_log_with_deadline(
            &docker::command(&agent_run_args(&plan)),
            &log,
            spec.timeout,
            &asked,
        )?;
        // Killing the `docker run` client leaves the container running.
        if matches!(finished, Finished::TimedOut | Finished::Stopped) {
            docker::stop(&agent, 10);
        }
        Ok(Ran {
            finished,
            output: fs::read_text(&log)?.unwrap_or_default(),
        })
    })();
    docker::remove_force(&agent);
    docker::remove_force(&proxy);
    docker::network_remove(&net);
    ran
}

impl WriteHuman for AgentRunReceipt {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let exit = self
            .exit_code
            .map_or_else(|| "-".to_owned(), |c| c.to_string());
        let outcome = serde_json::to_value(self.outcome)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        writeln!(w, "{} {} {outcome} exit={exit}", self.run_id, self.agent)?;
        writeln!(w, "{}", self.output)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/run.rs"]
mod tests;
