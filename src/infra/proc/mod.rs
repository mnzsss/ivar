//! The subprocess boundary. Nothing else in this crate touches
//! `std::process`.
//!
//! `ivar` shells out for exactly three reasons, and they want different
//! things from this module:
//!
//! - **git mutations and anything touching a remote** (ADR-0001 §3). Short,
//!   quiet, and their output is *evidence* — a failing `git clone`'s stderr is
//!   the sentence that ends up inside a [`Failure`](crate::error::Failure). So
//!   they [`capture`].
//! - **a repo's setup script**. Long (a `pnpm install` is minutes), noisy, and
//!   the user wants to watch it. Capturing would leave them staring at a frozen
//!   line, which is the failure the predecessor's own setup runner was written
//!   to avoid — it inherits the terminal on purpose. So they [`inherit`].
//! - **a provider process speaking a line-oriented protocol** — `claude -p
//!   --output-format stream-json`, `opencode run`. Nobody is watching it and
//!   there is no final answer to capture: the whole point is the lines that
//!   arrive *while it runs*, so a caller can parse and react to them as they
//!   come rather than discovering the transcript only once the process is
//!   already dead. So they [`stream`].
//!
//! All three block the calling thread for as long as they're asked to wait.
//! There is no async runtime in this crate, and adding one is an
//! architectural change, not a dependency bump.
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
use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, ChildStderr, ChildStdout, Command as StdCommand, Stdio};

use camino::Utf8PathBuf;

use crate::error::{Failure, FixAction};

mod ports;

pub use ports::{find_listening_ports, find_ports_for_program};

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

/// Spawn `command` with stdout piped for incremental reading while it runs,
/// stdin `/dev/null`, and stderr piped so it can be drained and explained if
/// the child fails.
///
/// [`capture`] buffers both streams and only hands them back once the process
/// is dead; that is exactly wrong for a caller parsing a provider's line
/// protocol, which needs each line as it arrives, not the whole transcript
/// after the fact. A [`portable_pty`](https://docs.rs/portable-pty)-backed
/// PTY was rejected for the same reason `session::start` uses one for a human
/// and this doesn't: a PTY interleaves and reflows what it displays, which
/// destroys line boundaries in a protocol that depends on them.
///
/// Stdin is `/dev/null`, for the same reason [`capture`] sets it: a child
/// nobody is watching interactively must never sit blocked on a prompt only a
/// human could answer.
pub fn stream(command: &Command) -> Result<Stream, Error> {
    let mut child = command
        .to_std()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| spawn_error(command, source))?;

    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(stdout), Some(stderr)) => (stdout, stderr),
        // Unreachable in practice: both were just requested as `Stdio::piped()`
        // above. Handled as a spawn error rather than `expect`/`unwrap` (the
        // crate warns on both) because there is nowhere safer to report it.
        _ => {
            return Err(spawn_error(
                command,
                io::Error::other("child stdout/stderr missing after a piped spawn"),
            ));
        }
    };

    Ok(Stream {
        command: command.clone(),
        child,
        stdout: BufReader::new(stdout),
        stderr,
        captured_stderr: String::new(),
    })
}

/// A child spawned by [`stream`]: readable line by line while it runs, with a
/// wait-for-exit for once the caller is done draining it.
///
/// `stdout` piped through a [`BufReader`] rather than [`capture`]'s "read it
/// all" is the entire capability this type exists to add — see the module
/// doc comment.
#[derive(Debug)]
pub struct Stream {
    /// Kept only so a failure inside [`Self::wait`] can render through
    /// [`spawn_error`], the same `Error::Spawn` shape [`capture`] and
    /// [`inherit`] already produce, rather than a second error type.
    command: Command,
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    captured_stderr: String,
}

impl Stream {
    /// The next line of stdout, blocking until one arrives or the stream
    /// ends. `Ok(None)` is end of stream — the child closed stdout, which
    /// happens at or before exit, so [`Self::wait`] is what to call next, not
    /// this again.
    ///
    /// The trailing line ending is stripped (`\n`, or `\r\n` with both
    /// removed), matching [`decode`]'s trimming for [`capture`]'s output.
    pub fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let bytes_read = self.stdout.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Some(line))
    }

    /// Block for the child to exit. Returns its exit code, `None` for a
    /// signal death — the same shape [`inherit`] returns, because both
    /// answer exactly one question: how did the process end.
    ///
    /// Also drains whatever the child wrote to stderr and decodes it lossily,
    /// like [`capture`], so [`Self::stderr`] has something to explain a
    /// failure with. Safe to call more than once, like the underlying
    /// [`Child::wait`].
    pub fn wait(&mut self) -> Result<Option<i32>, Error> {
        let mut raw_stderr = Vec::new();
        // A child that closed stderr (the normal case at exit) makes this
        // return `Ok(0)` rather than block; one still open would block here,
        // which is the known limitation the plan's safeguards accept: a hung
        // child hangs the caller, visibly rather than silently.
        let _ = self.stderr.read_to_end(&mut raw_stderr);
        self.captured_stderr = decode(&raw_stderr);

        let status = self
            .child
            .wait()
            .map_err(|source| spawn_error(&self.command, source))?;
        Ok(status.code())
    }

    /// The child's stderr, decoded and trimmed like [`capture`]'s. Empty
    /// until [`Self::wait`] has drained it.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.captured_stderr
    }
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
#[path = "../../../tests/unit/infra/proc.rs"]
mod tests;
