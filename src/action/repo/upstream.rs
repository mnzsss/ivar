//! `ivar repo upstream <repo> <url>` — manage remote upstream.

use crate::action::{Ctx, Done};
use crate::error::{Failure, Outcome};

#[derive(Debug, Clone)]
pub struct UpstreamInput {
    pub repo: String,
    pub url: String,
}

pub fn upstream(_ctx: &Ctx, _input: UpstreamInput) -> Outcome<Done> {
    Err(Failure::blocked(
        "repo.upstream.not_implemented",
        "repo upstream: not implemented yet",
    ))
}
