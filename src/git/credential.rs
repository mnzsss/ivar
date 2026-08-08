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
//! username=oauth2
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
    pub fn write(&self, mut stdout: impl Write) -> io::Result<()> {
        let w = &mut stdout;
        writeln!(w, "protocol={}", self.protocol)?;
        writeln!(w, "host={}", self.host)?;
        writeln!(w, "port={}", self.port)?;
        writeln!(w, "username={}", self.username)?;
        writeln!(w, "password={}", self.password)?;
        writeln!(w, "path={}", self.path)?;
        match self.approval {
            Approval::Approved => writeln!(w)?, // blank line = end of output
            Approval::Rejected => writeln!(w, "approval=rejected")?,
        }
        Ok(())
    }
}

/// Run the credential helper: read stdin, respond on stdout.
///
/// This is the entry point when git invokes `!ivar git-credential`. It reads
/// the credential request, populates username/password from the GitHub auth
/// cascade, and writes the response back.
pub fn run() -> io::Result<()> {
    let stdin = io::stdin().lock();
    let cred = Credential::read(stdin)?;

    // Only handle GitHub hosts — let git's own helpers deal with everything else.
    if !cred.host.contains("github") {
        // No response means git will try its next helper.
        return Ok(());
    }

    // Populate credentials using the GitHub auth cascade.
    let token_result = crate::infra::github::get_token();

    let mut response = cred.clone();
    match token_result {
        Ok(token) => {
            response.username = "x-access-token".to_owned();
            response.password = token;
        }
        Err(_) => {
            // Leave username/password empty — git will fail with its own message.
            // We don't degrade silently.
        }
    }

    response.write(io::stdout().lock())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn read_parses_the_protocol_until_the_blank_line() {
        let input = b"protocol=https\nhost=github.com\nusername=oauth2\n\n";
        let cred = Credential::read(&input[..]).unwrap();

        assert_eq!(cred.protocol, "https");
        assert_eq!(cred.host, "github.com");
        assert_eq!(cred.username, "oauth2");
        assert!(cred.password.is_empty());
    }

    #[test]
    fn read_ignores_unknown_keys_and_stops_at_the_separator() {
        let input = b"protocol=https\nhost=github.com\nunknown=whatever\n\n";
        let cred = Credential::read(&input[..]).unwrap();

        assert_eq!(cred.host, "github.com");
        // Everything after the blank line is a fresh request, not this one.
        let second = b"protocol=http\nhost=example.com\n\n";
        let parsed = Credential::read(&second[..]).unwrap();
        assert_eq!(parsed.protocol, "http");
        assert_eq!(parsed.host, "example.com");
    }

    #[test]
    fn write_emits_the_protocol_format_with_trailing_blank_line() {
        let cred = Credential {
            protocol: "https".to_owned(),
            host: "github.com".to_owned(),
            username: "x-access-token".to_owned(),
            password: "ghp_secret".to_owned(),
            ..Credential::default()
        };

        let mut out = Vec::new();
        cred.write(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains("protocol=https\n"));
        assert!(text.contains("host=github.com\n"));
        assert!(text.contains("username=x-access-token\n"));
        assert!(text.contains("password=ghp_secret\n"));
        assert!(
            text.ends_with("\n\n"),
            "protocol output ends with a blank line"
        );
    }

    #[test]
    fn approved_credential_ends_with_a_blank_line() {
        let cred = Credential {
            protocol: "https".to_owned(),
            host: "github.com".to_owned(),
            approval: Approval::Approved,
            ..Credential::default()
        };

        let mut out = Vec::new();
        cred.write(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with("\n\n"), "approved ends with the separator");
    }
}
