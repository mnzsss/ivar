//! `ivar skill sync` — materialise hall skills to native targets.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

pub fn sync(_ctx: &Ctx) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.sync.not_implemented",
        "skill sync: not implemented yet",
    ))
}
