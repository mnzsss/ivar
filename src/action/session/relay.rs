//! `ivar session relay` — relay session info (4-line output contract).

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct RelayInput {
    pub session: Option<String>,
}

pub fn relay(_ctx: &Ctx, _input: RelayInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "session.relay.not_implemented",
        "session relay: not implemented yet",
    ))
}
