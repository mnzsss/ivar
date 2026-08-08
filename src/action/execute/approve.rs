//! `ivar feature execute approve` — transition AwaitingApproval → Approved.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct ApproveInput {
    pub feature: String,
    pub workstream: String,
}

pub fn approve(_ctx: &Ctx, _input: ApproveInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "execute.approve.not_implemented",
        "feature execute approve: not implemented yet",
    ))
}
