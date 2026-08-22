use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::feature::{RunId, RunReceipt};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::feature::run;

#[derive(Debug, Clone)]
pub struct StatusInput {
    pub feature: String,
    pub history: bool,
    pub run: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub receipts: Vec<RunReceipt>,
}
impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        for receipt in &self.receipts {
            writeln!(w, "{} {}", receipt.id, receipt.status)?;
        }
        Ok(())
    }
}

pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let receipts = if let Some(id) = input.run {
        RunReceipt::find(&layout, &feature, &RunId::new(id)?)?
            .into_iter()
            .collect()
    } else if input.history {
        run::history(&layout, &feature)?
    } else {
        RunReceipt::read(&layout, &feature)?.into_iter().collect()
    };
    if receipts.is_empty() {
        return Err(Failure::blocked(
            "execute.run_missing",
            "no matching run receipt exists",
        ));
    }
    Ok(Report::new(StatusOutcome { receipts }))
}
