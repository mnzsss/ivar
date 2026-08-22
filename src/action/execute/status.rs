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
            writeln!(w, "Run {}: {}", receipt.id, receipt.status)?;
            writeln!(
                w,
                "  plan: {} ({})",
                receipt.plan_path, receipt.plan_fingerprint
            )?;
            writeln!(w, "  provenance: {}", receipt.provenance)?;
            writeln!(
                w,
                "  recovery: {}",
                match receipt.status {
                    crate::domain::feature::RunStatus::Blocked =>
                        "resume with `execute start --resume`",
                    crate::domain::feature::RunStatus::Diverged => {
                        "accept the approved plan with `execute accept-revision`"
                    }
                    _ => "none",
                }
            )?;
            let evidence =
                receipt
                    .final_diff
                    .as_ref()
                    .map_or("no final filesystem evidence", |diff| {
                        if diff.is_empty() {
                            "final filesystem evidence: no changes"
                        } else {
                            "final filesystem evidence: changes recorded"
                        }
                    });
            writeln!(w, "  evidence: {evidence}")?;
        }
        Ok(())
    }
}

pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    super::import_legacy(&layout, &feature, layout.plan_dir(&feature).join("plan.md"))?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::domain::feature::{RunBaseline, RunStatus};
    use crate::domain::name::SessionId;
    use crate::domain::provider::Provider;

    #[test]
    fn human_output_includes_receipt_recovery_plan_evidence_and_provenance() {
        let mut receipt = RunReceipt::start(
            RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
            FeatureName::new("checkout").unwrap(),
            "plans/checkout/plan.md",
            "plan-fingerprint",
            RunBaseline::empty(),
            SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
            Provider::ClaudeCode,
            "2026-01-01T00:00:00Z",
        );
        receipt.status = RunStatus::Blocked;
        let mut output = Vec::new();

        StatusOutcome {
            receipts: vec![receipt],
        }
        .write_human(&mut output)
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("plan: plans/checkout/plan.md (plan-fingerprint)"));
        assert!(output.contains("provenance: native"));
        assert!(output.contains("recovery: resume with `execute start --resume`"));
        assert!(output.contains("evidence: no final filesystem evidence"));
    }
}
