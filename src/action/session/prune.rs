//! `ivar session prune` — remove stale sessions.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

pub fn prune(_ctx: &Ctx) -> Outcome<Done> {
    Err(Failure::blocked(
        "session.prune.not_implemented",
        "session prune: not implemented yet",
    ))
}
