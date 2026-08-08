//! `ivar skill detach <skill>` — convert an external skill to an authored one.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct DetachInput {
    pub skill: String,
}

pub fn detach(_ctx: &Ctx, _input: DetachInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.detach.not_implemented",
        "skill detach: not implemented yet",
    ))
}
