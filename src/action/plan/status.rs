//! `ivar plan status <plan-path>` — show approval gate status.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct StatusInput {
    pub plan_path: String,
}

pub fn status(_ctx: &Ctx, _input: StatusInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "plan.status.not_implemented",
        "plan status: not implemented yet",
    ))
}
