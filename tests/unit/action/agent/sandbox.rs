#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_tool_that_cannot_start_is_named_without_its_prompt() {
    let prompt = format!("Evaluate the hall\n{}", "x".repeat(5_000));
    let command =
        Command::new("ivar-no-such-tool-anywhere").args(["-p".to_owned(), prompt.clone()]);
    let error = proc::exec(&command)
        .map_err(without_args)
        .expect_err("no binary");

    let failure: Failure = error.into();
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
