use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::session::lookup;
use crate::action::{Ctx, discover_hall};
use crate::domain::feature::{ApprovalState, Feature, Gate, RunReceipt};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::hash;
use crate::store::feature::run;

use super::snapshot;

#[derive(Debug, Clone)]
pub struct StartInput {
    pub feature: String,
    pub plan: String,
    pub resume: bool,
    pub restart: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartOutcome {
    pub feature: FeatureName,
    pub receipt: RunReceipt,
    pub receipt_path: Utf8PathBuf,
}
impl WriteHuman for StartOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Run {} is {}", self.receipt.id, self.receipt.status)
    }
}

pub fn start(ctx: &Ctx, input: StartInput) -> Outcome<StartOutcome> {
    if input.resume && input.restart {
        return Err(Failure::blocked(
            "execute.resume_restart_conflict",
            "--resume and --restart cannot be used together",
        ));
    }
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let plan = ctx.resolve(Utf8Path::new(&input.plan));
    let session = lookup::resolve(&layout, None, Some(feature.as_str()))?;
    let state = session.state.ok_or_else(|| {
        Failure::blocked(
            "execute.session_state_missing",
            "feature session has no state record",
        )
    })?;
    let feature_record = Feature::read(&layout, &feature)?.ok_or_else(|| {
        Failure::blocked(
            "feature.missing",
            format!("feature `{feature}` does not exist"),
        )
    })?;
    let approvals = ApprovalState::read(&layout, &feature)?.unwrap_or_else(ApprovalState::fresh);
    let fingerprint = hash::file(&plan)?;
    if approvals
        .record(Gate::Plan)
        .is_none_or(|record| record.artifact_fingerprint.as_deref() != Some(&fingerprint))
    {
        return Err(Failure::blocked(
            "execute.plan_not_approved",
            "the supplied plan is not the currently approved plan",
        ));
    }
    let now = rfc3339_now();
    if let Some(mut receipt) = RunReceipt::read(&layout, &feature)? {
        if input.restart && receipt.holds_lock() {
            receipt.interrupt(now.clone())?;
            receipt.write(&layout)?;
            run::archive_current(&layout, &feature)?;
        } else if input.resume {
            receipt.resume(session.id, state.provider, now)?;
            receipt.write(&layout)?;
            return Ok(Report::new(StartOutcome {
                receipt_path: run::current_path(&layout, &feature),
                feature,
                receipt,
            }));
        } else if receipt.holds_lock() {
            return Err(Failure::blocked(
                "execute.run_active",
                format!("run {} is {}", receipt.id, receipt.status),
            ));
        } else {
            run::archive_current(&layout, &feature)?;
        }
    }
    let worktrees = feature_record
        .promotions
        .keys()
        .map(|repo| {
            (
                repo.to_string(),
                layout.repo_worktree(repo, &feature_record.branch),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let baseline = snapshot::baseline(&worktrees)?;
    let receipt = RunReceipt::start(
        crate::domain::feature::RunId::new(uuid::Uuid::new_v4().to_string())?,
        feature.clone(),
        plan,
        fingerprint,
        baseline,
        session.id,
        state.provider,
        now,
    );
    receipt.write(&layout)?;
    Ok(Report::new(StartOutcome {
        receipt_path: run::current_path(&layout, &feature),
        feature,
        receipt,
    }))
}
