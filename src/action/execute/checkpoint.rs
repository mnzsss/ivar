use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::feature::RunReceipt;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::fs;
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

/// The wave numbers declared by `### Wave N` headings in a plan, in plan order.
///
/// Fenced code blocks are skipped so an example heading inside one does not
/// count. A trailing wave-complete marker or title after the number is ignored.
pub(crate) fn plan_waves(plan: &str) -> Vec<u32> {
    let mut in_fence = false;
    let mut waves = Vec::new();
    for line in plan.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let Some(heading) = trimmed.strip_prefix('#') else {
            continue;
        };
        let Some(rest) = heading
            .trim_start_matches('#')
            .strip_prefix(|c: char| c.is_whitespace())
            .and_then(|rest| rest.trim_start().strip_prefix("Wave"))
            .and_then(|rest| rest.strip_prefix(|c: char| c.is_whitespace()))
        else {
            continue;
        };
        let rest = rest.trim_start();
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .map_or(rest, |end| &rest[..end]);
        if let Ok(number) = digits.parse::<u32>()
            && !waves.contains(&number)
        {
            waves.push(number);
        }
    }
    waves
}

/// Refuse a wave number the plan does not declare, naming the waves it does.
///
/// A plan with no `Wave N` headings (or one that cannot be read) cannot name
/// its waves, so it is not second-guessed here.
fn ensure_wave_in_plan(plan: &camino::Utf8Path, wave: u32) -> Result<(), Failure> {
    let Some(text) = fs::read_text(plan)? else {
        return Ok(());
    };
    let waves = plan_waves(&text);
    if waves.is_empty() || waves.contains(&wave) {
        return Ok(());
    }
    let listed = waves
        .iter()
        .map(|w| format!("Wave {w}"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(Failure::blocked(
        "execute.wave_not_in_plan",
        format!("plan `{plan}` has no Wave {wave}; its waves are: {listed}"),
    ))
}

pub fn checkpoint(ctx: &Ctx, input: CheckpointInput) -> Outcome<CheckpointOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let mut receipt =
        RunReceipt::read(&layout, &feature)?.ok_or_else(|| super::run_missing(&feature))?;
    let (session, provider) = super::resolve_coordinator(&layout, &feature, &receipt)?;
    ensure_wave_in_plan(&ctx.resolve(&receipt.plan_path), input.wave)?;
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
