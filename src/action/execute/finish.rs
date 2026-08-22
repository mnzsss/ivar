use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::session::lookup;
use crate::action::{Ctx, discover_hall};
use crate::domain::feature::{CoordinatorReport, RunOutcome, RunReceipt};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::feature::run;

use super::snapshot;

#[derive(Debug, Clone)]
pub struct FinishInput {
    pub feature: String,
    pub plan: String,
    pub report_json: String,
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct FinishOutcome {
    pub feature: FeatureName,
    pub receipt: RunReceipt,
    pub receipt_path: Utf8PathBuf,
}
impl WriteHuman for FinishOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Run {} is {}", self.receipt.id, self.receipt.status)
    }
}

pub fn finish(ctx: &Ctx, input: FinishInput) -> Outcome<FinishOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let plan = ctx.resolve(Utf8Path::new(&input.plan));
    super::import_legacy(&layout, &feature, plan.clone())?;
    let session = lookup::resolve(&layout, None, Some(feature.as_str()))?;
    let state = session.state.ok_or_else(|| {
        Failure::blocked(
            "execute.session_state_missing",
            "feature session has no state record",
        )
    })?;
    let report: CoordinatorReport = serde_json::from_str(
        &fs::read_text(&ctx.resolve(Utf8Path::new(&input.report_json)))?.ok_or_else(|| {
            Failure::blocked("execute.report_missing", "report JSON does not exist")
        })?,
    )
    .map_err(|error| Failure::blocked("execute.report_invalid", error.to_string()))?;
    report.validate()?;
    let outcome = RunOutcome::parse(&input.outcome)?;
    let mut receipt = RunReceipt::read(&layout, &feature)?
        .ok_or_else(|| Failure::blocked("execute.run_missing", "no current run receipt exists"))?;
    let plan_fingerprint = hash::file(&plan)?;
    let diff = snapshot::diff(&receipt.baseline)?;
    let now = rfc3339_now();
    if plan_fingerprint != receipt.plan_fingerprint {
        receipt.diverge(
            plan_fingerprint,
            Some(report),
            session.id,
            state.provider,
            now,
        )?;
        receipt.write(&layout)?;
        return Err(Failure::blocked(
            "execute.plan_diverged",
            "the approved plan changed while this run was active",
        ));
    }
    if outcome == RunOutcome::Blocked {
        receipt.block(report, diff, session.id, state.provider, now)?;
        receipt.write(&layout)?;
    } else {
        receipt.terminate(outcome, report, diff, session.id, state.provider, now)?;
        receipt.write(&layout)?;
        run::archive_current(&layout, &feature)?;
    }
    Ok(Report::new(FinishOutcome {
        receipt_path: run::current_path(&layout, &feature),
        feature,
        receipt,
    }))
}
