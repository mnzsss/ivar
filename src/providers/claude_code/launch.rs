use crate::infra::proc::Command;
use crate::providers::{Capabilities, LaunchContract};

const CAPABILITIES: Capabilities = Capabilities {
    supports_resume: true,
    supports_review: true,
    interactive: true,
};

const CONTRACT: LaunchContract = LaunchContract {
    binary: "claude",
    capabilities: CAPABILITIES,
};

#[must_use]
pub const fn contract() -> LaunchContract {
    CONTRACT
}

pub fn start_command(resume: bool) -> Command {
    let command = Command::new(CONTRACT.binary);
    if resume {
        command.arg("--continue")
    } else {
        command
    }
}
