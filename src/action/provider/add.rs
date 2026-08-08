//! `ivar provider add <name>` — register a provider.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct AddInput {
    pub name: String,
}

pub fn add(_ctx: &Ctx, _input: AddInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "provider.add.not_implemented",
        "provider add: not implemented yet",
    ))
}
