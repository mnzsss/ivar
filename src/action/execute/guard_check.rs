//! `ivar feature execute guard-check` — check write contract.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct GuardCheckInput {
    pub feature: Option<String>,
    pub session: Option<String>,
    pub path: Option<String>,
}

pub fn guard_check(_ctx: &Ctx, _input: GuardCheckInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "execute.guard_check.not_implemented",
        "feature execute guard-check: not implemented yet",
    ))
}
