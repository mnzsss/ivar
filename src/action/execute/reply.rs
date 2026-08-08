//! `ivar feature execute reply` — reply to blocked workstream.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct ReplyInput {
    pub feature: Option<String>,
    pub session: Option<String>,
    pub message: String,
}

pub fn reply(_ctx: &Ctx, _input: ReplyInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "execute.reply.not_implemented",
        "feature execute reply: not implemented yet",
    ))
}
