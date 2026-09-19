//! Rendering an action's `Outcome`/`Report`/`Failure` as bytes, and the exit
//! code that goes with it. Split out of `run` so that file stays under the
//! line-count ceiling `tests/architecture.rs`'s sibling size check enforces.

use std::io;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::batch::BatchItemResult;
use crate::error::{Failure, Outcome, Palette, Report, WriteHuman};
use crate::infra::term;

/// Render a [`Report`]'s exit code: clean is `0`, warnings present is `1`.
/// [`Report::is_clean`] is the one switch — see the module doc comment.
fn exit_code_for<T>(report: &Report<T>) -> ExitCode {
    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// The palette for whatever is being written to stderr — failures and warnings.
///
/// Reads the decision `main` already primed, so the flag is honoured without
/// being threaded through every dispatch arm.
fn stderr_palette() -> Palette {
    Palette::from_decision(term::colour_for(term::Stream::Stderr, None))
}

/// Render whatever an action returned, and pick the exit code.
///
/// The one place the success half of an [`Outcome`] is turned into bytes, so
/// every verb renders identically and neither `--json` nor the human text can
/// acquire a second, hand-written formatting path. `respond_failure` is the
/// error half of the same pair.
pub(super) fn respond<T>(
    result: Outcome<T>,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode
where
    T: Serialize + WriteHuman,
{
    match result {
        Ok(report) => {
            let exit = exit_code_for(&report);
            if json {
                let _ = write_json(stdout, &report);
            } else {
                // The value is never painted: it is data, and the --json
                // surface shows the same strings raw.
                let _ = report.value.write_human(stdout);
                let palette = stderr_palette();
                for warning in &report.warnings {
                    let _ = warning.write_painted(stderr, &palette);
                }
            }
            exit
        }
        Err(failure) => respond_failure(&failure, json, stdout, stderr),
    }
}

pub(super) fn respond_batch<T>(
    items: Vec<BatchItemResult<T>>,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode
where
    T: Serialize + WriteHuman,
{
    let mut any_failed = false;
    let mut any_warn = false;
    for item in items {
        match item.outcome {
            Ok(report) => {
                if !report.is_clean() {
                    any_warn = true;
                }
                if json {
                    let _ = write_json(stdout, &report);
                } else {
                    let palette = stderr_palette();
                    for warning in &report.warnings {
                        let _ = warning.write_painted(stderr, &palette);
                    }
                    let _ = report.value.write_human(stdout);
                }
            }
            Err(failure) => {
                any_failed = true;
                if json {
                    let _ = write_json(stdout, &failure);
                } else {
                    let _ = failure.write_painted(stderr, &stderr_palette());
                }
            }
        }
    }
    if any_failed {
        ExitCode::from(2)
    } else if any_warn {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

pub(super) fn respond_failure(
    failure: &Failure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    if json {
        let _ = write_json(stdout, &failure);
    } else {
        let _ = failure.write_painted(stderr, &stderr_palette());
    }
    ExitCode::from(2)
}

/// The envelope written when a value cannot be serialized at all — the one
/// JSON a caller can still parse when nothing else rendered.
pub(crate) const RENDER_FAILED_JSON: &str = r#"{"ok":false,"kind":"failed","code":"cli.render_failed","what":"could not render JSON output"}"#;

/// The `--json` surface: the value's `Serialize` form, one line, to `w`.
fn write_json(w: &mut impl io::Write, value: &impl Serialize) -> io::Result<()> {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| RENDER_FAILED_JSON.to_owned());
    writeln!(w, "{rendered}")
}

/// The real process's current directory, as a [`Utf8PathBuf`]. Falls back to
/// `.` if it cannot be read or is not valid UTF-8 — vanishingly rare, and
/// `Ctx::resolve` still does the right thing with a relative fallback.
pub(super) fn current_dir() -> Utf8PathBuf {
    let raw = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Utf8PathBuf::from_path_buf(raw).unwrap_or_else(|_| Utf8PathBuf::from("."))
}
