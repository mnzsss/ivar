//! `ivar repo setup <repo>` — run setup script for one repo.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct SetupInput {
    pub repo: String,
}

pub fn setup(_ctx: &Ctx, _input: SetupInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "repo.setup.not_implemented",
        "repo setup: not implemented yet",
    ))
}
