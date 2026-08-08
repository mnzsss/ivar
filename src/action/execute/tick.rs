//! `ivar feature execute tick` — find ready workstreams and launch them.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct TickInput {
    pub feature: String,
}

pub fn tick(_ctx: &Ctx, _input: TickInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "execute.tick.not_implemented",
        "feature execute tick: not implemented yet",
    ))
}
