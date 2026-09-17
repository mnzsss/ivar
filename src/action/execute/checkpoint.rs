use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::feature::RunReceipt;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::feature::run;

#[derive(Debug, Clone)]
pub struct CheckpointInput {
    pub feature: String,
    pub wave: u32,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckpointOutcome {
    pub feature: FeatureName,
    pub receipt: RunReceipt,
    pub receipt_path: Utf8PathBuf,
}

impl WriteHuman for CheckpointOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let wave = self
            .receipt
            .checkpoints
            .last()
            .and_then(|checkpoint| checkpoint.wave.as_ref());
        match wave {
            Some(wave) => writeln!(w, "Run {} recorded wave {}", self.receipt.id, wave.number),
            None => writeln!(w, "Run {} is {}", self.receipt.id, self.receipt.status),
        }
    }
}

pub fn checkpoint(ctx: &Ctx, input: CheckpointInput) -> Outcome<CheckpointOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let mut receipt = RunReceipt::read(&layout, &feature)?
        .ok_or_else(|| Failure::blocked("execute.run_missing", "no current run receipt exists"))?;
    let (session, provider) = super::resolve_coordinator(&layout, &feature, &receipt)?;
    receipt.checkpoint_wave(input.wave, input.summary, session, provider, rfc3339_now())?;
    receipt.write(&layout)?;
    Ok(Report::new(CheckpointOutcome {
        receipt_path: run::current_path(&layout, &feature),
        feature,
        receipt,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/checkpoint.rs"]
mod tests;
