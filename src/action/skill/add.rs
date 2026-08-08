//! `ivar skill add <repo> [--path] [--ref]` — install an external skill.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

/// What `ivar skill add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    pub repo: String,
    pub path: Option<String>,
    pub ref_: Option<String>,
}

pub fn add(_ctx: &Ctx, _input: AddInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "skill.add.not_implemented",
        "skill add: not implemented yet",
    ))
}
