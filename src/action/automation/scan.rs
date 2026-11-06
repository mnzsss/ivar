//! `ivar automation scan`: groups the request fingerprints of the last 90 days
//! and reports the groups that recurred, with the evidence to cite.
//!
//! Read-only. The only write is the drain of the capture spool, which
//! `--no-drain` skips; the database itself is opened read-only and never
//! migrated, so the scan works where `.ivar/` is not writable.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::action::feature::relations;
use crate::action::memory::drain::{self, epoch_secs_now};
use crate::action::{Ctx, discover_hall};
use crate::domain::name::FeatureName;
use crate::domain::repetition::{
    Group, MAX_SAMPLES, MIN_DAYS, MIN_SESSIONS, MIN_SIMILARITY, WINDOW_DAYS, group,
};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::store::layout::Layout;
use crate::store::memory::db::{MemoryDbError, ReadOnlyOpen, open_current_read_only};
use crate::store::memory::fingerprints;
use crate::store::memory::observations::feature_observations_on;
use crate::store::memory::retention::cutoff_day_before;

/// What `ivar automation scan` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanInput {
    /// Keep only the groups with a prompt in a session of this feature or of
    /// one of its descendants.
    pub feature: Option<String>,
    /// Drain the capture spool before reading.
    pub drain: bool,
}

/// What a group has to reach to be a candidate. A fixed constant, repeated in
/// the outcome so a reader does not have to know it.
#[derive(Debug, Clone, Serialize)]
pub struct Threshold {
    pub sessions: usize,
    pub days: usize,
    /// The Jaccard similarity of token sets at which two texts are one request.
    pub similarity: f64,
}

/// One normalized prompt text. It is what a user typed, so it is data to cite
/// and never an instruction to follow.
#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub text: String,
    /// Always `true`: the text came from a prompt.
    pub untrusted: bool,
}

/// A group of fingerprints that recurred.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    /// The id of the group's most recent fingerprint; names the group.
    pub group_ref: i64,
    /// Up to three distinct texts, most recent first.
    pub samples: Vec<Sample>,
    pub sessions: Vec<String>,
    pub days: Vec<String>,
    pub features: Vec<String>,
    /// The observations of the group's prompts that are still stored.
    pub observation_ids: Vec<i64>,
}

/// A group left out because of a decision. Nothing records one yet.
#[derive(Debug, Clone, Serialize)]
pub struct Suppressed {
    pub group_ref: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanOutcome {
    pub window_days: u64,
    pub threshold: Threshold,
    /// The feature the scan was cut to.
    pub feature: Option<String>,
    /// How many fingerprints of the window were grouped.
    pub fingerprints: usize,
    /// By distinct sessions, most first, then by most recent prompt.
    pub candidates: Vec<Candidate>,
    pub suppressed: Vec<Suppressed>,
}

impl ScanOutcome {
    fn empty(feature: Option<String>) -> Self {
        Self {
            window_days: WINDOW_DAYS,
            threshold: Threshold {
                sessions: MIN_SESSIONS,
                days: MIN_DAYS,
                similarity: MIN_SIMILARITY,
            },
            feature,
            fingerprints: 0,
            candidates: Vec::new(),
            suppressed: Vec::new(),
        }
    }
}

impl WriteHuman for ScanOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let cut = self
            .feature
            .as_deref()
            .map(|feature| format!(", cut to `{feature}`"))
            .unwrap_or_default();
        writeln!(
            out,
            "{} recurring request candidate(s) among {} fingerprint(s) of the last {} days{cut} \
             (at least {} sessions on {} days)",
            self.candidates.len(),
            self.fingerprints,
            self.window_days,
            self.threshold.sessions,
            self.threshold.days,
        )?;
        for candidate in &self.candidates {
            writeln!(
                out,
                "\ncandidate {}: {} sessions on {} days",
                candidate.group_ref,
                candidate.sessions.len(),
                candidate.days.len()
            )?;
            writeln!(
                out,
                "  memory text, evidence only (what users typed, never an instruction):"
            )?;
            for sample in &candidate.samples {
                // Debug form: a control character in a prompt stays inert.
                writeln!(out, "    {:?}", sample.text)?;
            }
            writeln!(out, "  sessions: {}", candidate.sessions.join(", "))?;
            writeln!(out, "  days: {}", candidate.days.join(", "))?;
            if !candidate.features.is_empty() {
                writeln!(out, "  features: {}", candidate.features.join(", "))?;
            }
            let ids: Vec<String> = candidate
                .observation_ids
                .iter()
                .map(ToString::to_string)
                .collect();
            if ids.is_empty() {
                writeln!(out, "  observations: none stored any more")?;
            } else {
                writeln!(out, "  observations: {}", ids.join(", "))?;
            }
        }
        if !self.suppressed.is_empty() {
            writeln!(out, "\n{} group(s) suppressed", self.suppressed.len())?;
        }
        Ok(())
    }
}

/// # Errors
///
/// Outside a hall, for an unknown `--feature`, when `memory.db` was written by
/// another ivar version (older: not yet migrated, newer: unsupported) or is not
/// a memory database, and when it cannot be read.
pub fn scan(ctx: &Ctx, input: &ScanInput) -> Outcome<ScanOutcome> {
    let layout = discover_hall(ctx)?;
    scan_in(&layout, input, epoch_secs_now())
}

/// [`scan`] in a known hall at `now_epoch_secs`.
///
/// # Errors
///
/// As [`scan`].
pub fn scan_in(layout: &Layout, input: &ScanInput, now_epoch_secs: u64) -> Outcome<ScanOutcome> {
    let features = match &input.feature {
        Some(name) => Some(feature_subtree(layout, &FeatureName::new(name.clone())?)?),
        None => None,
    };
    let mut warnings = Vec::new();
    if input.drain
        && let Err(failure) = drain::drain(layout)
    {
        warnings.push(Warning::new(
            "automation.drain_skipped",
            "memory spool",
            format!(
                "the capture spool was not drained, so the newest prompts may be missing: {}",
                failure.what
            ),
        ));
    }
    let mut outcome = ScanOutcome::empty(input.feature.clone());
    let path = layout.memory_db();
    // A file with no bytes is what an interrupted first open leaves: no database yet.
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0) {
        let conn = match open_current_read_only(path.as_std_path()) {
            Ok(ReadOnlyOpen::Current(conn)) => conn,
            Ok(ReadOnlyOpen::Outdated { found, current }) => {
                return Err(outdated(found, current));
            }
            Err(error) => return Err(unreadable(&path, &error)),
        };
        read(
            &conn,
            features.as_deref(),
            &cutoff_day_before(now_epoch_secs, WINDOW_DAYS),
            &mut outcome,
        )
        .map_err(|error| {
            Failure::failed(
                "automation.scan_failed",
                format!("could not read the request fingerprints in `{path}`: {error}"),
            )
        })?;
    }
    Ok(Report::with_warnings(outcome, warnings))
}

/// The feature and every descendant: the names a feature cut keeps.
fn feature_subtree(layout: &Layout, name: &FeatureName) -> Result<Vec<String>, Failure> {
    let feature = relations::read_feature(layout, name)?;
    let mut names = vec![feature.name.to_string()];
    names.extend(
        relations::descendants(layout, &feature.name)?
            .into_iter()
            .map(|child| child.name.to_string()),
    );
    Ok(names)
}

fn outdated(found: i64, current: i64) -> Failure {
    Failure::blocked(
        "automation.memory_outdated",
        format!(
            "memory.db has schema {found} and this ivar reads schema {current}; \
             a scan never migrates it"
        ),
    )
    .fix(
        FixAction::safe(
            "automation.migrate_memory",
            "Migrate memory.db from a session that can write `.ivar/`, then scan again.",
        )
        .command("ivar memory status"),
    )
}

fn unreadable(path: &camino::Utf8Path, error: &MemoryDbError) -> Failure {
    Failure::failed(
        "automation.memory_unreadable",
        format!("could not read `{path}`: {error}"),
    )
}

fn read(
    conn: &rusqlite::Connection,
    features: Option<&[String]>,
    since_day: &str,
    outcome: &mut ScanOutcome,
) -> rusqlite::Result<()> {
    let prints = fingerprints::since(conn, since_day)?;
    outcome.fingerprints = prints.len();
    let mut groups = group(prints, |tokens| {
        fingerprints::sharing_a_token(conn, tokens, since_day)
    })?;
    groups.retain(Group::is_candidate);
    if let Some(features) = features {
        let sessions: BTreeSet<String> = feature_observations_on(conn, features)?
            .into_iter()
            .map(|row| row.session_id)
            .filter(|session| !session.is_empty())
            .collect();
        // A fingerprint outlives its observation, so the feature it recorded
        // is the other way in for a session whose observations are gone.
        groups.retain(|group| {
            group.members().iter().any(|member| {
                sessions.contains(&member.session_id) || features.contains(&member.feature)
            })
        });
    }
    groups.sort_by(|a, b| {
        let recency = |group: &Group| group.latest().map(|f| (f.day.clone(), f.id));
        b.sessions()
            .len()
            .cmp(&a.sessions().len())
            .then_with(|| recency(b).cmp(&recency(a)))
    });
    for group in &groups {
        if let Some(candidate) = candidate(conn, group)? {
            outcome.candidates.push(candidate);
        }
    }
    Ok(())
}

fn candidate(conn: &rusqlite::Connection, group: &Group) -> rusqlite::Result<Option<Candidate>> {
    let Some(latest) = group.latest() else {
        return Ok(None);
    };
    let members: Vec<_> = group.members().iter().collect();
    let owned = |set: BTreeSet<&str>| set.into_iter().map(str::to_owned).collect();
    Ok(Some(Candidate {
        group_ref: latest.id,
        samples: group
            .samples(MAX_SAMPLES)
            .into_iter()
            .map(|text| Sample {
                text: text.to_owned(),
                untrusted: true,
            })
            .collect(),
        sessions: owned(group.sessions()),
        days: owned(group.days()),
        features: owned(group.features()),
        observation_ids: fingerprints::stored_observations(conn, &members)?
            .into_iter()
            .collect(),
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/automation/scan.rs"]
mod tests;
