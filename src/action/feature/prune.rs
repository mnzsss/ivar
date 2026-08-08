//! `ivar feature prune` — delete features with merged branches.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

pub fn prune(_ctx: &Ctx) -> Outcome<Done> {
    Err(Failure::blocked(
        "feature.prune.not_implemented",
        "feature prune: not implemented yet",
    ))
}
