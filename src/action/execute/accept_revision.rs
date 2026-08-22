use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::session::lookup;
use crate::action::{Ctx, discover_hall};
use crate::domain::feature::{ApprovalState, Gate, RunReceipt};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::hash;
use crate::store::feature::run;

#[derive(Debug, Clone)]
pub struct AcceptRevisionInput {
    pub feature: String,
    pub plan: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct AcceptRevisionOutcome {
    pub feature: FeatureName,
    pub receipt: RunReceipt,
    pub receipt_path: Utf8PathBuf,
}
impl WriteHuman for AcceptRevisionOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Run {} now awaits resume", self.receipt.id)
    }
}

pub fn accept_revision(ctx: &Ctx, input: AcceptRevisionInput) -> Outcome<AcceptRevisionOutcome> {
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
    let fingerprint = hash::file(&plan)?;
    let approvals = ApprovalState::read(&layout, &feature)?.unwrap_or_else(ApprovalState::fresh);
    if approvals
        .record(Gate::Plan)
        .is_none_or(|record| record.artifact_fingerprint.as_deref() != Some(&fingerprint))
    {
        return Err(Failure::blocked(
            "execute.plan_not_approved",
            "the supplied plan is not currently approved",
        ));
    }
    let mut receipt = RunReceipt::read(&layout, &feature)?
        .ok_or_else(|| Failure::blocked("execute.run_missing", "no current run receipt exists"))?;
    receipt.accept_revision(fingerprint, session.id, state.provider, rfc3339_now())?;
    receipt.write(&layout)?;
    Ok(Report::new(AcceptRevisionOutcome {
        receipt_path: run::current_path(&layout, &feature),
        feature,
        receipt,
    }))
}
