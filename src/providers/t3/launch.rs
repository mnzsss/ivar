use crate::infra::proc::Command;
use crate::providers::{Capabilities, LaunchContract};

const CONTRACT: LaunchContract = LaunchContract {
    binary: "t3",
    capabilities: Capabilities {
        supports_resume: false,
        supports_review: false,
        interactive: false,
        initial_prompt: false,
        resume_prompt: false,
    },
};

#[must_use]
pub const fn contract() -> LaunchContract {
    CONTRACT
}

#[must_use]
pub fn start_command() -> Command {
    Command::new(CONTRACT.binary)
}
