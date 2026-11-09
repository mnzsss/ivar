#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io;

use super::*;

#[test]
fn a_tool_that_cannot_start_is_named_without_its_prompt() {
    let prompt = format!("Evaluate the hall\n{}", "x".repeat(5_000));
    let spawn = proc::Error::Spawn {
        command: format!("ivar-no-such-tool-anywhere -p {prompt}"),
        program: "ivar-no-such-tool-anywhere".to_owned(),
        source: io::Error::from(io::ErrorKind::NotFound),
    };

    let failure: Failure = without_args(spawn).into();

    assert!(
        failure.what.contains("`ivar-no-such-tool-anywhere …`"),
        "{}",
        failure.what
    );
    assert!(
        !failure.what.contains("Evaluate the hall"),
        "{}",
        failure.what
    );
}
