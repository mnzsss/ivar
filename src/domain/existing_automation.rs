//! Existing automations: what an `accepted` decision looks like once the
//! automation has been running for a while.
//!
//! Pure. The state, the usage and the **retirable** mark are all derived from
//! what `memory.db` already holds (request fingerprints and tool
//! observations); nothing is stored. Where those rows come from is the store's
//! business; this module only decides what they mean.
//!
//! * a **use** is a `session.tool` observation, in a session that asked for the
//!   request, whose input cites the artifact: a script or procedure by its
//!   hall-relative path, a skill or agent by its name (the file stem). Only a
//!   procedure is used by reading it; a script that was only opened was not run;
//! * the **state** is `in use` (a use in the last 30 days), `unused` (the
//!   request came back in the last 30 days and nothing used the artifact) or
//!   `no requests` (it did not come back);
//! * it is **retirable** when the form can be deleted (`script`, `skill`,
//!   `procedure`), the decision is at least 90 days old (or last renewed that
//!   long ago), and no fingerprint matched in those 90 days. The age gate is
//!   what lets `renew` clear the mark "for another 90 days": the window counts
//!   from `max(window start, updated_at)`, so it is never shorter than 90 days
//!   when the mark is set. `unused` alone never makes it retirable, and an
//!   `agent` never is.

use std::collections::BTreeSet;
use std::fmt;

use camino::Utf8Path;

use crate::domain::automation::{Decision, Form, clean_artifact};
use crate::domain::repetition::{Fingerprint, Group};
use crate::domain::script::ScriptScope;

/// How many days back a request or a use counts for the state.
pub const STATE_DAYS: u64 = 30;
/// How many days without a matching request make an automation retirable.
pub const RETIRE_DAYS: u64 = 90;

/// What the recent past says about an existing automation. It serializes as
/// [`State::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// At least one use in the last 30 days.
    InUse,
    /// The request came back in the last 30 days and nothing used it.
    Unused,
    /// The request did not come back in the last 30 days.
    NoRequests,
}

impl serde::Serialize for State {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl State {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InUse => "in use",
            Self::Unused => "unused",
            Self::NoRequests => "no requests",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One fingerprint that matched an accepted decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    /// Empty when the prompt was typed outside an `ivar` session.
    pub session_id: &'a str,
    /// The UTC date of the prompt, `YYYY-MM-DD`.
    pub day: &'a str,
}

impl<'a> From<&'a Fingerprint> for Request<'a> {
    fn from(fingerprint: &'a Fingerprint) -> Self {
        Self {
            session_id: &fingerprint.session_id,
            day: &fingerprint.day,
        }
    }
}

/// The fingerprints of `groups` that ask for what `decision` automates: every
/// member of every group the decision matches, so a new wording of the request
/// still counts.
pub fn matching_fingerprints<'a>(
    decision: &'a Decision,
    groups: &'a [Group],
) -> impl Iterator<Item = &'a Fingerprint> + 'a {
    groups
        .iter()
        .filter(|group| decision.matches(group.texts()))
        .flat_map(Group::members)
}

/// The sessions whose requests fall in the last 30 days, the ones the state
/// is judged on. A prompt typed outside a session has none.
#[must_use]
pub fn recent_sessions<'a>(requests: &[Request<'a>], days: Days<'_>) -> BTreeSet<&'a str> {
    requests
        .iter()
        .filter(|request| request.day >= days.state_since)
        .map(|request| request.session_id)
        .filter(|session| !session.is_empty())
        .collect()
}

/// The days that bound the judgement, all `YYYY-MM-DD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Days<'a> {
    /// The first day of the last 30 days.
    pub state_since: &'a str,
    /// The first day of the last 90 days.
    pub window_since: &'a str,
}

/// What [`assess`] concludes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub state: State,
    pub retirable: bool,
    /// Sessions of the last 30 days that asked for the request and ran a tool
    /// citing the artifact, sorted.
    pub sessions_with_use: Vec<String>,
    /// Sessions of the last 30 days that asked for it and cited nothing, sorted.
    pub sessions_without_use: Vec<String>,
}

/// Judges an accepted `decision`. `requests` are the fingerprints of the last
/// 90 days that match it; `used_in` says whether a session ran a tool citing
/// the artifact.
#[must_use]
pub fn assess(
    decision: &Decision,
    requests: &[Request<'_>],
    days: Days<'_>,
    mut used_in: impl FnMut(&str) -> bool,
) -> Assessment {
    let asked_recently = requests
        .iter()
        .any(|request| request.day >= days.state_since);
    let (with_use, without_use): (Vec<&str>, Vec<&str>) = recent_sessions(requests, days)
        .into_iter()
        .partition(|session| used_in(session));
    let state = if !with_use.is_empty() {
        State::InUse
    } else if asked_recently {
        State::Unused
    } else {
        State::NoRequests
    };
    let retirable = decision.form.is_some_and(|form| form != Form::Agent)
        && day_of(&decision.updated_at).is_some_and(|day| day <= days.window_since)
        && requests
            .iter()
            .all(|request| request.day < days.window_since);
    Assessment {
        state,
        retirable,
        sessions_with_use: with_use.into_iter().map(str::to_owned).collect(),
        sessions_without_use: without_use.into_iter().map(str::to_owned).collect(),
    }
}

/// The UTC date of an RFC 3339 timestamp, `None` when the value cannot hold
/// one: a decision with an unreadable date is never retirable.
fn day_of(timestamp: &str) -> Option<&str> {
    timestamp
        .get(..10)
        .filter(|day| day.as_bytes().first().is_some_and(u8::is_ascii_digit))
}

/// The text a tool input has to contain to cite the artifact: the clean
/// hall-relative path of a script or procedure, the name of a skill or agent.
/// `None` when the artifact has no usable name.
#[must_use]
pub fn cited_as(form: Form, artifact: &str) -> Option<String> {
    match form {
        Form::Script | Form::Procedure => clean_artifact(artifact).ok(),
        Form::Skill | Form::Agent => {
            let path = Utf8Path::new(artifact);
            let stem = path.file_stem()?;
            // A skill is a folder holding `SKILL.md`: its name is the folder's.
            let name = if stem.eq_ignore_ascii_case("SKILL") {
                path.parent()?.file_name()?
            } else {
                stem
            };
            Some(name.to_owned())
        }
    }
}

/// Whether `text` cites `needle`, the [`cited_as`] text of an artifact: it
/// holds it as a whole word, so `deploy` is not found in `redeploy`,
/// `deploy-all` or `deploy.sh`.
#[must_use]
pub fn cites(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    text.match_indices(needle).any(|(at, _)| {
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let mut after_chars = text.get(at + needle.len()..).unwrap_or_default().chars();
        let next = after_chars.next();
        let extension = next == Some('.') && after_chars.next().is_some_and(is_word_char);
        !before.is_some_and(is_word_char) && !next.is_some_and(is_word_char) && !extension
    })
}

/// Whether the body of a `session.tool` observation is a use of an artifact of
/// this `form`: it cites `needle` in the tool's input. The body is JSON:
/// `input` for a tool that ran, `path` for a read, which keeps only the path.
/// A read uses only a procedure, which is followed by reading it; a script,
/// skill or agent that was only opened was not run. A body that is not JSON
/// cites nothing.
#[must_use]
pub fn body_cites(body: &str, form: Form, needle: &str) -> bool {
    let Ok(body) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let keys: &[&str] = if form == Form::Procedure {
        &["input", "path"]
    } else {
        &["input"]
    };
    keys.iter()
        .filter_map(|key| body.get(*key))
        .any(|value| value_cites(value, needle))
}

fn value_cites(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => cites(text, needle),
        serde_json::Value::Array(items) => items.iter().any(|item| value_cites(item, needle)),
        serde_json::Value::Object(fields) => fields.values().any(|item| value_cites(item, needle)),
        _ => false,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// The hall folders that hold personal, gitignored artifacts: personal
/// scripts and personal skills.
const PERSONAL_FOLDERS: [&str; 2] = [".ivar/scripts-local/", ".ivar/skills-local/"];

/// Where the artifact lives: personal inside `.ivar/scripts-local/` or
/// `.ivar/skills-local/`, team everywhere else.
#[must_use]
pub fn scope_of(artifact: &str) -> ScriptScope {
    let path = clean_artifact(artifact).unwrap_or_else(|_| artifact.to_owned());
    if PERSONAL_FOLDERS
        .iter()
        .any(|folder| path.starts_with(folder))
    {
        ScriptScope::Personal
    } else {
        ScriptScope::Team
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/existing_automation.rs"]
mod tests;
