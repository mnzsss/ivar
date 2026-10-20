//! `ivar agent feedback`: a verdict on one agent run.

use crate::action::{Ctx, discover_hall};
use crate::domain::agent::receipt::{AgentRunReceipt, Rating, RunFeedback};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report};
use crate::store;

#[derive(Debug)]
pub struct FeedbackInput {
    pub run_id: String,
    pub rating: String,
    pub comment: Option<String>,
}

/// # Errors
///
/// Returns a [`Failure`] for an invalid rating, an unknown run, or a receipt
/// that cannot be read or written.
pub fn feedback(ctx: &Ctx, input: FeedbackInput) -> Outcome<AgentRunReceipt> {
    let layout = discover_hall(ctx)?;
    let rating: Rating = input
        .rating
        .parse()
        .map_err(|e: String| Failure::failed("agent.invalid_rating", e))?;
    let mut receipt = store::agent::find_receipt(&layout, &input.run_id)?.ok_or_else(|| {
        Failure::blocked(
            "agent.run_not_found",
            format!("no agent run `{}`", input.run_id),
        )
    })?;
    receipt.feedback = Some(RunFeedback {
        rating,
        comment: input.comment,
        recorded_at: rfc3339_now(),
    });
    store::agent::write_receipt(&layout, &receipt)?;
    Ok(Report::new(receipt))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/feedback.rs"]
mod tests;
