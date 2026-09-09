use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::feature::RunReceipt;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::store::feature::run;

#[derive(Debug, Clone)]
pub struct InterruptInput {
    pub feature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterruptOutcome {
    pub feature: FeatureName,
    pub receipt: RunReceipt,
    pub receipt_path: Utf8PathBuf,
}

impl WriteHuman for InterruptOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Run {} is {}", self.receipt.id, self.receipt.status)
    }
}

pub fn interrupt(ctx: &Ctx, input: InterruptInput) -> Outcome<InterruptOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let mut receipt = RunReceipt::read(&layout, &feature)?
        .ok_or_else(|| Failure::blocked("execute.run_missing", "no current run receipt exists"))?;
    if !receipt.holds_lock() {
        return Err(Failure::blocked(
            "execute.run_not_active",
            format!("run {} is {} and holds no lock", receipt.id, receipt.status),
        )
        .fix(FixAction::safe(
            "execute.inspect_run",
            "Inspect the run with `ivar feature execute status <feature>`.",
        )));
    }
    let _ = super::resolve_coordinator(&layout, &feature, &receipt)?;
    let now = rfc3339_now();
    receipt.interrupt(now)?;
    receipt.write(&layout)?;
    run::archive_current(&layout, &feature)?;
    Ok(Report::new(InterruptOutcome {
        receipt_path: run::current_path(&layout, &feature),
        feature,
        receipt,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/interrupt.rs"]
mod tests;
