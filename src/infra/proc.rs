//! The subprocess boundary. Nothing else in this crate touches
//! `std::process`.
//!
//! `ivar` shells out for exactly two reasons, and they want different things
//! from this module:
//!
//! - **git mutations and anything touching a remote** (ADR-0001 §3). Short,
//!   quiet, and their output is *evidence* — a failing `git clone`'s stderr is
//!   the sentence that ends up inside a [`Failure`](crate::error::Failure). So
//!   they [`capture`].
//! - **a repo's setup script**. Long (a `pnpm install` is minutes), noisy, and
//!   the user wants to watch it. Capturing would leave them staring at a frozen
//!   line, which is the failure the predecessor's own setup runner was written
//!   to avoid — it inherits the terminal on purpose. So they [`inherit`].
//!
//! Both are blocking. There is no async runtime in this crate, and adding one
//! is an architectural change, not a dependency bump.
//!
//! # A non-zero exit is data, not an error
//!
//! [`Error`] has one variant and it means *the program never ran*: not found,
//! not executable, working directory gone. Everything a program says by
//! finishing comes back in [`Output`] for the caller to interpret, because
//! "non-zero" means something different in every program `ivar` runs — `git
//! worktree list` exits 128 for a path that is not a repository, which is a
//! perfectly good answer to a question, and `git merge --ff-only` exits 1 for
//! "would not fast-forward", which is the thing being tested.
//!
//! Folding that into `Err` would push every caller into digging an exit code
//! back out of an error type it only wrapped a moment earlier.
//!
//! # Output is decoded lossily
//!
//! `stdout` and `stderr` come back as `String` via
//! [`String::from_utf8_lossy`], never as a UTF-8 error. A subprocess that
//! writes a stray byte is not a case any user can act on, and the only thing
//! this crate does with captured output is put it in a message. Failing there
//! would replace a readable error with an unreadable one.

use std::ffi::OsStr;
use std::io;
use std::process::{Command as StdCommand, Stdio};

use camino::Utf8PathBuf;

use crate::error::{Failure, FixAction};

/// A program to run: what, with which arguments, from where, with which
/// environment overrides.
///
/// Built with the chaining setters, then handed to [`capture`] or [`inherit`].
/// Holding the invocation as a value (rather than driving
/// `std::process::Command` in place) is what lets both runners share one
/// [`Command::display`] — the string that ends up in an error message has to be
/// the command the user can read back, and deriving it separately at each call
/// site is how it stops matching what actually ran.
#[derive(Debug, Clone)]
pub struct Command {
    program: String,
    args: Vec<String>,
    cwd: Option<Utf8PathBuf>,
    env: Vec<(String, String)>,
}

impl Command {
    /// A command that runs `program`, inheriting the parent environment.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    /// Append one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments, in order.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Run from `dir` instead of the parent's working directory.
    ///
    /// Every caller in this crate sets this. Inheriting the process's working
    /// directory would reintroduce exactly the ambient dependency
    /// [`crate::action::Ctx`] exists to remove.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<Utf8PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Set one environment variable, on top of the inherited environment.
    ///
    /// Setting a variable to the empty string is meaningful and is *not* the
    /// same as unsetting it — `GIT_ASKPASS=` is how git is told to stop looking
    /// for an askpass helper, and it is why this module offers no `env_remove`.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The invocation as a human reads it back, for error messages.
    ///
    /// Deliberately not shell-quoted: it names the program and its arguments
    /// for someone reading a failure, and quoting would suggest it is safe to
    /// paste into a shell verbatim when an argument contains a space.
    #[must_use]
    pub fn display(&self) -> String {
        let mut rendered = self.program.clone();
        for arg in &self.args {
            rendered.push(' ');
            rendered.push_str(arg);
        }
        rendered
    }

    /// The program to run. Exposed so a consumer that spawns through its own
    /// mechanism (e.g. `portable-pty`, which is not a `std::process` runner)
    /// can rebuild the invocation from the same value this module would have
    /// run — the one source of truth for what runs.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The arguments, in order.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.args
    }

    /// The environment overrides.
    #[must_use]
    pub fn envs(&self) -> &[(String, String)] {
        &self.env
    }

    /// The working directory, if set.
    #[must_use]
    pub fn working_dir(&self) -> Option<&Utf8PathBuf> {
        self.cwd.as_ref()
    }

    /// Everything both runners configure identically.
    fn to_std(&self) -> StdCommand {
        let mut command = StdCommand::new(&self.program);
        command.args(self.args.iter().map(OsStr::new));
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// What a program said by finishing.
///
/// See the module doc comment: a non-zero [`Self::code`] is an answer, not an
/// error.
#[derive(Debug, Clone)]
pub struct Output {
    /// The exit code, or `None` when the process was killed by a signal.
    pub code: Option<i32>,
    /// Captured standard output, trailing whitespace trimmed, decoded lossily.
    pub stdout: String,
    /// Captured standard error, trailing whitespace trimmed, decoded lossily.
    pub stderr: String,
}

impl Output {
    /// Whether the program exited zero. A signal death is not success.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// The most useful sentence this program produced, for an error message:
    /// its stderr, or its stdout when stderr is empty (plenty of programs
    /// report on the wrong stream), or a description of the exit itself when
    /// both are.
    #[must_use]
    pub fn diagnostic(&self) -> String {
        if !self.stderr.is_empty() {
            return self.stderr.clone();
        }
        if !self.stdout.is_empty() {
            return self.stdout.clone();
        }
        match self.code {
            Some(code) => format!("exited {code} with no output"),
            None => "killed by a signal".to_owned(),
        }
    }
}

/// Run `command` to completion, capturing both streams.
///
/// Stdin is `/dev/null`: a git invocation must never block on a credential
/// prompt nobody can see. The caller owns the meaning of the exit code — see
/// the module doc comment.
pub fn capture(command: &Command) -> Result<Output, Error> {
    let output = command
        .to_std()
        .stdin(Stdio::null())
        .output()
        .map_err(|source| spawn_error(command, source))?;

    Ok(Output {
        code: output.status.code(),
        stdout: decode(&output.stdout),
        stderr: decode(&output.stderr),
    })
}

/// Run `command` to completion with all three streams inherited, so the user
/// watches it happen. Returns the exit code (`None` for a signal death).
///
/// Stdin is inherited too, which is the deliberate difference from
/// [`capture`]: a setup script that needs an SSH passphrase or a `sudo`
/// password has to be able to ask.
pub fn inherit(command: &Command) -> Result<Option<i32>, Error> {
    let status = command
        .to_std()
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| spawn_error(command, source))?;

    Ok(status.code())
}

fn spawn_error(command: &Command, source: io::Error) -> Error {
    Error::Spawn {
        command: command.display(),
        program: command.program.clone(),
        source,
    }
}

/// Lossy, then trailing whitespace trimmed — every consumer of a captured
/// stream wants the sentence, not the newline the program ended it with.
fn decode(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).trim_end().to_owned()
}

/// The one thing that can go wrong that is not the program's own answer: it
/// never ran.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The program could not be started: not on `PATH`, not executable, or its
    /// working directory does not exist.
    #[error("could not run `{command}`: {source}")]
    Spawn {
        /// The full invocation, as [`Command::display`] renders it.
        command: String,
        /// The program alone, for the fix action.
        program: String,
        #[source]
        source: io::Error,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        let what = error.to_string();

        match error {
            Error::Spawn {
                program, source, ..
            } => Failure::blocked("proc.spawn_failed", what)
                .expected(format!("`{program}` to be installed and on PATH"))
                .actual(source.to_string())
                .fix(FixAction::safe(
                    "proc.install_program",
                    format!("Install `{program}`, or put it on PATH, then try again."),
                )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::error::Status;
    use crate::test_support::utf8_temp_dir;

    // -- capture: the exit code is data ---------------------------------------

    #[test]
    fn capture_returns_stdout_and_a_zero_code_on_success() {
        let output = capture(&Command::new("echo").arg("hello")).unwrap();

        assert!(output.success());
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout, "hello");
        assert_eq!(output.stderr, "");
    }

    /// If a non-zero exit were an `Err`, every caller that cares about *which*
    /// non-zero code it got would have to dig it back out of an error type.
    #[test]
    fn a_non_zero_exit_is_ok_not_err() {
        let output = capture(&Command::new("sh").arg("-c").arg("exit 3")).unwrap();

        assert!(!output.success());
        assert_eq!(output.code, Some(3));
    }

    #[test]
    fn capture_separates_the_two_streams() {
        let output = capture(
            &Command::new("sh")
                .arg("-c")
                .arg("printf out; printf err >&2"),
        )
        .unwrap();

        assert_eq!(output.stdout, "out");
        assert_eq!(output.stderr, "err");
    }

    #[test]
    fn captured_output_has_its_trailing_newline_trimmed() {
        let output = capture(&Command::new("sh").arg("-c").arg("printf 'line\\n\\n'")).unwrap();

        assert_eq!(output.stdout, "line");
    }

    #[test]
    fn invalid_utf8_is_decoded_lossily_rather_than_failing() {
        let output = capture(&Command::new("sh").arg("-c").arg("printf 'a\\377b'")).unwrap();

        assert!(output.success());
        assert!(
            output.stdout.contains('\u{fffd}'),
            "expected a replacement character, got {:?}",
            output.stdout
        );
    }

    // -- cwd and env ----------------------------------------------------------

    #[test]
    fn cwd_is_where_the_program_runs() {
        let (_guard, dir) = utf8_temp_dir();
        let canonical = dir.canonicalize_utf8().unwrap();

        let output = capture(&Command::new("pwd").cwd(&canonical)).unwrap();

        assert_eq!(output.stdout, canonical.as_str());
    }

    #[test]
    fn env_overrides_are_visible_to_the_program() {
        let output = capture(
            &Command::new("sh")
                .arg("-c")
                .arg("printf %s \"$IVAR_TEST_VALUE\"")
                .env("IVAR_TEST_VALUE", "set-by-the-test"),
        )
        .unwrap();

        assert_eq!(output.stdout, "set-by-the-test");
    }

    /// Setting a variable to the empty string is how git is told to stop
    /// looking for an askpass helper. It must reach the child as *set and
    /// empty*, not as absent — the two mean different things to git.
    #[test]
    fn an_empty_env_value_is_set_rather_than_unset() {
        let output = capture(
            &Command::new("sh")
                .arg("-c")
                .arg("if [ -n \"${IVAR_TEST_EMPTY+x}\" ]; then printf set; else printf unset; fi")
                .env("IVAR_TEST_EMPTY", ""),
        )
        .unwrap();

        assert_eq!(output.stdout, "set");
    }

    // -- the one error --------------------------------------------------------

    #[test]
    fn a_program_that_does_not_exist_is_a_spawn_error() {
        let error =
            capture(&Command::new("ivar-no-such-program-exists-anywhere")).expect_err("no binary");

        assert!(matches!(error, Error::Spawn { .. }));
    }

    #[test]
    fn spawn_failure_converts_to_a_blocked_failure_naming_the_program() {
        let error = capture(&Command::new("ivar-no-such-program-exists-anywhere").arg("--version"))
            .expect_err("no binary");

        let failure: Failure = error.into();
        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "proc.spawn_failed");
        assert!(
            failure
                .what
                .contains("ivar-no-such-program-exists-anywhere"),
            "message was: {}",
            failure.what
        );
        assert_eq!(failure.fix_actions.len(), 1);
        assert!(failure.fix_actions[0].safe);
    }

    #[test]
    fn a_missing_working_directory_is_a_spawn_error_too() {
        let (_guard, dir) = utf8_temp_dir();

        let error = capture(&Command::new("echo").cwd(dir.join("does-not-exist")))
            .expect_err("cwd does not exist");

        assert!(matches!(error, Error::Spawn { .. }));
    }

    // -- inherit --------------------------------------------------------------

    #[test]
    fn inherit_returns_the_exit_code() {
        assert_eq!(inherit(&Command::new("true")).unwrap(), Some(0));
        assert_eq!(
            inherit(&Command::new("sh").arg("-c").arg("exit 7")).unwrap(),
            Some(7)
        );
    }

    // -- display, diagnostic, availability ------------------------------------

    #[test]
    fn display_renders_the_invocation_a_human_reads_back() {
        let command = Command::new("git").arg("clone").arg("--bare").arg("url");

        assert_eq!(command.display(), "git clone --bare url");
    }

    #[test]
    fn diagnostic_prefers_stderr_then_stdout_then_the_exit_itself() {
        let with_stderr = Output {
            code: Some(1),
            stdout: "out".to_owned(),
            stderr: "err".to_owned(),
        };
        assert_eq!(with_stderr.diagnostic(), "err");

        let stdout_only = Output {
            code: Some(1),
            stdout: "out".to_owned(),
            stderr: String::new(),
        };
        assert_eq!(stdout_only.diagnostic(), "out");

        let silent = Output {
            code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(silent.diagnostic(), "exited 1 with no output");

        let signalled = Output {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!signalled.success());
        assert_eq!(signalled.diagnostic(), "killed by a signal");
    }
}
