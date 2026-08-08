//! `ivar skill update [skills...]` — update external skills to their tracked ref.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct UpdateInput {
    pub skills: Vec<String>,
}

pub fn update(_ctx: &Ctx, _input: UpdateInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.update.not_implemented",
        "skill update: not implemented yet",
    ))
}
