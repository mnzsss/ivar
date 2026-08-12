//! Git credential helper protocol.
//!
//! Register with git so it calls ivar for GitHub credentials:
//! ```text
//! git config --global credential.https://github.com.helper '!ivar git-credential'
//! ```
//!
//! The helper reads the credential protocol from stdin (key=value pairs ending
//! with a blank line) and writes back the response in the same format. It never
//! writes tokens to `.git/config` — that is why we register this instead of
//! storing credentials in the config file.
//!
//! # The operation argument
//!
//! git never runs a helper bare. It appends the operation it wants as the last
//! argument, so the registration above is invoked as `ivar git-credential get`
//! when git needs a credential and `ivar git-credential store` after the
//! transfer succeeds. A helper that accepts no operand therefore fails on
//! *every* invocation — including the ones whose output nobody reads, which is
//! how it stays invisible until it is printing an error in the middle of every
//! push.
//!
//! Only `get` produces output here. ivar keeps no credential store of its own —
//! the token is re-derived from the `gh`/`$GITHUB_TOKEN` cascade on demand — so
//! `store` and `erase` have nothing to record and nothing to forget, and an
//! operation this build has never heard of is ignored rather than refused, as
//! gitcredentials(7) requires.
//!
//! # Saying nothing is an answer
//!
//! git collects *all* configured helpers and asks them in order, stopping at
//! the first that returns a username and a password. It reads an empty
//! `username=` as a username that happens to be empty — set, not absent — so a
//! helper that answers with blanks silences every helper behind it. When this
//! one has no token, or the host is not GitHub, it writes nothing at all and
//! the user's own `gh` or keychain helper gets its turn.
//!
//! # Protocol
//!
//! Input  — lines of `key=value`, terminated by an empty line:
//! ```text
//! protocol=https
//! host=github.com
//!
//! ```
//!
//! Output — same format, plus any keys you want to set:
//! ```text
//! protocol=https
//! host=github.com
//! username=x-access-token
//! password=ghp_xxxxxxxxxxxx
//!
//! ```

use std::io::{self, BufRead, Write};

/// A parsed git credential request/response.
#[derive(Debug, Default, Clone)]
pub struct Credential {
    pub protocol: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub path: String,
    pub approval: Approval,
}

/// Whether the caller approved or rejected this credential.
#[derive(Debug, Default, Clone, Copy)]
pub enum Approval {
    #[default]
    Approved,
    Rejected,
}

/// What git is asking this helper to do.
///
/// The operation is git's last argument on every invocation. See the module
/// doc comment for why all three exist here when only one of them answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// git needs a credential.
    Get,
    /// git is reporting a credential that worked.
    Store,
    /// git is reporting a credential that did not.
    Erase,
    /// Something this build does not implement. Ignored, per
    /// gitcredentials(7) — a helper that refuses an operation it does not know
    /// turns the next git release into an error on every push.
    Unknown,
}

impl Operation {
    /// Read the operation off git's command line.
    ///
    /// A missing argument is taken as [`Operation::Get`]: git always passes
    /// one, so the only caller who can omit it is a human running the helper
    /// by hand to see what the token cascade answers — for whom `get` is the
    /// only useful reading.
    pub fn from_arg(arg: Option<&str>) -> Self {
        match arg {
            Some("get") | None => Self::Get,
            Some("store") => Self::Store,
            Some("erase") => Self::Erase,
            Some(_) => Self::Unknown,
        }
    }
}

impl Credential {
    /// Read a credential request from stdin, parse key=value lines until the
    /// blank separator line.
    pub fn read(stdin: impl BufRead) -> io::Result<Self> {
        let mut cred = Self::default();
        for line in stdin.lines() {
            let line = line?;
            if line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "protocol" => cred.protocol = value.to_owned(),
                    "host" => cred.host = value.to_owned(),
                    "port" => cred.port = value.to_owned(),
                    "username" => cred.username = value.to_owned(),
                    "password" => cred.password = value.to_owned(),
                    "path" => cred.path = value.to_owned(),
                    _ => {} // ignore unknown keys
                }
            }
        }
        Ok(cred)
    }

    /// Write this credential to stdout in git's credential protocol format.
    ///
    /// Fields with no value are omitted rather than written empty. `path=` is
    /// the one that bites: git does not read it as "no path", it fills the
    /// credential with an empty path and carries it into the request.
    pub fn write(&self, mut stdout: impl Write) -> io::Result<()> {
        let w = &mut stdout;
        for (key, value) in [
            ("protocol", &self.protocol),
            ("host", &self.host),
            ("port", &self.port),
            ("username", &self.username),
            ("password", &self.password),
            ("path", &self.path),
        ] {
            if value.is_empty() {
                continue;
            }
            writeln!(w, "{key}={value}")?;
        }
        match self.approval {
            Approval::Approved => writeln!(w)?, // blank line = end of output
            Approval::Rejected => writeln!(w, "approval=rejected")?,
        }
        Ok(())
    }
}

/// Run the credential helper: read stdin, respond on stdout.
///
/// This is the entry point when git invokes `!ivar git-credential <operation>`.
/// `operation` is whatever git appended; [`Operation::from_arg`] decides what
/// it means.
pub fn run(operation: Option<&str>) -> io::Result<()> {
    respond(
        Operation::from_arg(operation),
        io::stdin().lock(),
        io::stdout().lock(),
        || crate::infra::github::get_token().ok(),
    )
}

/// The helper, with its two edges as parameters: git on `stdin`/`stdout`, and
/// the GitHub auth cascade behind `token`.
///
/// `token` is a closure rather than a value so that `store` and `erase` — the
/// operations git runs on the happy path of every push — never shell out to
/// `gh` for a token they have no use for.
fn respond(
    operation: Operation,
    stdin: impl BufRead,
    mut stdout: impl Write,
    token: impl FnOnce() -> Option<String>,
) -> io::Result<()> {
    let request = Credential::read(stdin)?;

    // Everything but `get` is git telling us what happened, not asking. The
    // request is still parsed first: git writes one either way, and expects a
    // reader on the other end.
    if operation != Operation::Get {
        return Ok(());
    }

    // Only handle GitHub hosts — let git's own helpers deal with everything
    // else. No response means git moves on to the next helper.
    if !request.host.contains("github") {
        return Ok(());
    }

    // No token: say nothing, so the helper behind this one is still asked.
    // See the module doc comment — an empty answer is not an absent one.
    let Some(token) = token() else {
        return Ok(());
    };

    let response = Credential {
        username: "x-access-token".to_owned(),
        password: token,
        ..request
    };
    response.write(&mut stdout)
}

#[cfg(test)]
#[path = "../../tests/unit/git/credential.rs"]
mod tests;
