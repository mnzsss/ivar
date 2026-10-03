#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;

use super::*;
use crate::cli::root::ColorMode;

fn argv(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn try_parse_keeps_the_global_color_flag() {
    let cli = try_parse(&argv(&["ivar", "status", "--color", "never"])).unwrap();
    assert_eq!(cli.color, ColorMode::Never);
}

#[test]
fn help_is_a_stdout_error_whose_ansi_render_is_styled_and_plain_render_is_not() {
    let err = try_parse(&argv(&["ivar", "--help"])).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    assert!(
        !err.use_stderr(),
        "help goes to stdout, so stdout's decision colours it"
    );
    let styled = err.render().ansi().to_string();
    let plain = err.render().to_string();
    assert!(styled.contains('\x1b'), "STYLES must reach the help text");
    assert!(!plain.contains('\x1b'));
    assert!(plain.contains("Usage:"));
}

#[test]
fn a_usage_error_goes_to_stderr_with_exit_code_two() {
    let err = try_parse(&argv(&["ivar", "no-such-verb"])).unwrap_err();
    assert!(err.use_stderr());
    assert_eq!(err.exit_code(), 2);
}
