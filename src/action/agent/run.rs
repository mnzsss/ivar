//! `ivar agent run`: one sandboxed run of an agent, always ending in a receipt.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use uuid::Uuid;

use super::feature_access::{self, Delivered};
use super::notify;
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::agent::container::{
    ContainerPlan, Mount, agent_run_args, proxy_run_args, tinyproxy_conf, tinyproxy_filter,
};
use crate::domain::agent::images::{BASE_IMAGE, PROXY_IMAGE};
use crate::domain::agent::receipt::{self, AgentRunReceipt, Trigger, classify, scrub};
use crate::domain::agent::spec::{Access, AgentSpec, Launch};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::proc::{self, Finished};
use crate::infra::{docker, fs};
use crate::store;
use crate::store::agent_secrets::{AgentSecrets, render_env_file};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

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

struct Ran {
    finished: Finished,
    output: String,
}

struct Attempt {
    ran: Ran,
    secret_values: Vec<String>,
    delivered: Option<Delivered>,
}

/// # Errors
///
/// A [`Failure`] only when the hall or the agent file cannot be loaded, or
/// the receipt cannot be written; every other failure is a `failed` receipt.
pub fn run(ctx: &Ctx, input: RunInput) -> Outcome<AgentRunReceipt> {
    let RunInput { name, trigger } = input;
    let layout = discover_hall(ctx)?;
    let spec = store::agent::load(&layout, &name)?;
    let started_at = rfc3339_now();
    let run_id = receipt::run_id(&started_at, &Uuid::new_v4().simple().to_string()[..8]);
    let lock = match fs::try_lock_exclusive(&layout.agent_lock(&spec.name)) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            let skipped = AgentRunReceipt::skipped(run_id, spec.name.clone(), trigger, started_at);
            return finish(&layout, &spec, skipped);
        }
        Err(error) => {
            let failed = AgentRunReceipt::failed(
                run_id,
                spec.name.clone(),
                trigger,
                started_at,
                rfc3339_now(),
                Failure::from(error).what,
            );
            return finish(&layout, &spec, failed);
        }
    };
    let scratch = layout.agent_run_scratch(&run_id);
    let attempt = (|| -> Result<Attempt, Failure> {
        let secrets = AgentSecrets::read(&layout)?.select(&spec.secrets)?;
        let (ws, worktree) = workspace_for(ctx, &layout, &spec)?;
        let ran = execute(&spec, &run_id, &scratch, &secrets, ws)?;
        let delivered = worktree
            .map(|wt| feature_access::commit_and_preview(ctx, &layout, &spec, &wt, &run_id));
        Ok(Attempt {
            ran,
            secret_values: secrets.into_iter().map(|(_, v)| v).collect(),
            delivered,
        })
    })();
    let _ = fs::remove_path(&scratch);
    let receipt = match attempt {
        Err(failure) => AgentRunReceipt::failed(
            run_id,
            spec.name.clone(),
            trigger,
            started_at,
            rfc3339_now(),
            failure.what,
        ),
        Ok(attempt) => completed(&spec, run_id, trigger, started_at, &attempt),
    };
    let result = finish(&layout, &spec, receipt);
    drop(lock);
    result
}

fn completed(
    spec: &AgentSpec,
    run_id: String,
    trigger: Trigger,
    started_at: String,
    attempt: &Attempt,
) -> AgentRunReceipt {
    let values: Vec<&str> = attempt.secret_values.iter().map(String::as_str).collect();
    let output = scrub(&attempt.ran.output, &values);
    let tool = match spec.launch {
        Launch::Tool { tool, .. } => Some(tool),
        Launch::Image { .. } => None,
    };
    let blocked =
        attempt.ran.finished == Finished::Stopped || tool.is_some_and(|t| t.is_blocked(&output));
    let (exit_code, timed_out) = match attempt.ran.finished {
        Finished::Exited(code) => (code, false),
        Finished::TimedOut => (None, true),
        Finished::Stopped => (None, false),
    };
    let mut receipt = AgentRunReceipt {
        outcome: classify(exit_code, timed_out, blocked),
        exit_code,
        cost_usd: tool.and_then(|t| t.cost_usd(&output)),
        output,
        ended_at: rfc3339_now(),
        ..AgentRunReceipt::skipped(run_id, spec.name.clone(), trigger, started_at)
    };
    if let Some(d) = &attempt.delivered {
        receipt.diff = d.diff.as_deref().map(|t| scrub(t, &values));
        receipt.delivery_fingerprint.clone_from(&d.fingerprint);
        receipt.delivery_error = d.error.as_deref().map(|t| scrub(t, &values));
    }
    receipt
}

/// The single point that records a run.
pub(crate) fn finish(
    layout: &Layout,
    spec: &AgentSpec,
    mut receipt: AgentRunReceipt,
) -> Outcome<AgentRunReceipt> {
    notify::deliver(spec, &mut receipt);
    store::agent::write_receipt(layout, &receipt)?;
    Ok(Report::new(receipt))
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
