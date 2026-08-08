//! `ivar skill doctor` — health diagnostics with fix_actions.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

pub fn doctor(_ctx: &Ctx) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.doctor.not_implemented",
        "skill doctor: not implemented yet",
    ))
}
