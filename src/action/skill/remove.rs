//! `ivar skill remove <skill>` — remove a skill.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct RemoveInput {
    pub skill: String,
}

pub fn remove(_ctx: &Ctx, _input: RemoveInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.remove.not_implemented",
        "skill remove: not implemented yet",
    ))
}
