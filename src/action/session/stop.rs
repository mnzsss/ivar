//! `ivar session stop [session]` — stop a session.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct StopInput {
    pub session: Option<String>,
}

pub fn stop(_ctx: &Ctx, _input: StopInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "session.stop.not_implemented",
        "session stop: not implemented yet",
    ))
}
