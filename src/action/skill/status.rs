//! `ivar skill status` — show skill installation state.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

pub fn status(_ctx: &Ctx) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.status.not_implemented",
        "skill status: not implemented yet",
    ))
}
