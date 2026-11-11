//! `ivar agent run` without `--wait`: the same run, in a background process.

use std::io;

use serde::Serialize;

use super::sync::current_ivar_bin;
use crate::action::{Ctx, discover_hall};
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::proc;
use crate::store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Started {
    pub agent: String,
}

/// Starts `ivar agent run <name> --wait` detached from the terminal. The agent
/// file is loaded first, so an invalid one fails here and not in the background.
///
/// # Errors
///
/// A [`crate::error::Failure`] when the hall or the agent file cannot be
/// loaded, or the background process cannot be spawned.
pub fn start(ctx: &Ctx, name: &str) -> Outcome<Started> {
    let layout = discover_hall(ctx)?;
    let spec = store::agent::load(&layout, name)?;
    let command = proc::Command::new(current_ivar_bin()?.as_str())
        .args(["agent", "run", spec.name.as_str(), "--wait"])
        .cwd(ctx.cwd.clone())
        .process_group();
    proc::detach(&command)?;
    Ok(Report::new(Started { agent: spec.name }))
}

impl WriteHuman for Started {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Started `{agent}` in the background. `ivar agent logs {agent}` prints its run once it ends.",
            agent = self.agent
        )
    }
}
