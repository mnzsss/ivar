//! `memory_review` and `ivar memory review`: what a feature wrote to memory,
//! its gaps and the durable pages its diff may have made stale, and the
//! confirmation that gates `deliver apply`.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Value, json};

use super::capture::CaptureEnv;
use super::drain;
use super::events::{MemoryEventScope, record_memory_event};
use super::mcp::feature_usage_session;
use super::status::SpoolStatus;
use crate::action::feature::select::resolve_single_feature;
use crate::action::feature::{base, relations};
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::feature::{Feature, Gate, GateState, WorktreeState};
pub use crate::domain::memory_review::Verdict;
use crate::domain::memory_review::{
    Gap, ObservationFact, PageAction, StalePage, USAGE_EVENT, Unattributed, WrittenPage,
    analysis_approved_at, explain_gaps, gaps, is_delivered, is_durable_page, missing_verdicts,
    queried_before_approval, stale_pages, written_pages,
};
use crate::domain::name::FeatureName;
use crate::domain::observation::SourceEvent;
use crate::domain::session::{epoch_secs_from_rfc3339, rfc3339_from_epoch_secs};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::store::layout::Layout;
use crate::store::memory::anchors::{
    GraphAttach, anchored_files, attach_graph, attach_graph_untouched, broken_anchors, detach_graph,
};
use crate::store::memory::db::{MemoryDb, MemoryDbError, ReadOnlyOpen, open_current_read_only};
use crate::store::memory::observations::{
    FeatureObservation, feature_observations, feature_observations_on, unattributed_page_writes,
};
use crate::store::memory::usage::{session_queries, unattributed_queries};

const MAX_RECORDED_ENTRIES: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct ReviewReport {
    pub feature: String,
    pub written: Vec<WrittenPage>,
    /// Writes and queries from a server with no session that named no feature,
    /// inside the feature's window. Possibly another feature's: informational,
    /// and they never close a gap.
    #[serde(skip_serializing_if = "Unattributed::is_empty")]
    pub unattributed: Unattributed,
    pub gaps: Vec<Gap>,
    pub stale: Vec<StalePage>,
    /// Anchors that were already broken and that the diff did not touch:
    /// informational, they need no verdict and never block the confirmation.
    pub preexisting_broken: Vec<StalePage>,
    pub warnings: Vec<String>,
    pub last: Option<ReviewRecord>,
    #[serde(skip)]
    incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewRecord {
    pub source_event: String,
    pub occurred_at: String,
    pub verdicts: Vec<(String, Verdict)>,
    pub not_required: Option<String>,
    pub reason: Option<String>,
    pub heads: BTreeMap<String, String>,
    pub written: Vec<String>,
    pub stale: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewState {
    Reviewed { heads: BTreeMap<String, String> },
    Skipped,
    Missing,
}

/// What `ivar memory review --confirm` attests: the verdict on each possibly
/// stale page and, for a feature that wrote no page, why none was needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Confirmation {
    pub verdicts: Vec<(String, Verdict)>,
    pub not_required: Option<String>,
}

/// # Errors
///
/// Outside a hall, for an unknown feature, or when memory cannot be read.
pub fn review(ctx: &Ctx, feature: Option<String>) -> Outcome<ReviewReport> {
    review_or_confirm(ctx, feature, None)
}

/// The read-only review, or with a `confirmation` the same checks and record
/// as the `memory_review` MCP tool's `confirm`, through [`confirm_in`]. The
/// event carries the process's session, if any, and the feature named.
///
/// # Errors
///
/// As [`review`]; with a confirmation also those of [`confirm_in`].
pub fn review_or_confirm(
    ctx: &Ctx,
    feature: Option<String>,
    confirmation: Option<Confirmation>,
) -> Outcome<ReviewReport> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(resolve_single_feature(
        ctx,
        feature,
        "Select a feature to review",
    )?)?;
    let Some(confirmation) = confirmation else {
        return review_in(&layout, &name);
    };
    let scope = MemoryEventScope {
        session_id: CaptureEnv::from_process().session_id.unwrap_or_default(),
        feature: name.to_string(),
        parent_feature: String::new(),
    };
    confirm_in(
        &layout,
        &scope,
        &name,
        &confirmation.verdicts,
        confirmation.not_required,
    )
}

impl WriteHuman for ReviewReport {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        writeln!(out, "memory review of `{}`", self.feature)?;
        if self.written.is_empty() {
            writeln!(out, "written: none")?;
        }
        for page in &self.written {
            writeln!(
                out,
                "written: {} ({:?}, {:?})",
                page.path, page.action, page.source
            )?;
        }
        for page in &self.unattributed.written {
            writeln!(
                out,
                "unattributed write: {} ({:?}; no session or feature named, may not be this feature's)",
                page.path, page.action
            )?;
        }
        if self.unattributed.queries > 0 {
            writeln!(
                out,
                "unattributed queries: {} (no session or feature named, may not be this feature's)",
                self.unattributed.queries
            )?;
        }
        for gap in &self.gaps {
            writeln!(out, "gap {}: {}", gap.kind.as_str(), gap.detail)?;
        }
        if self.stale.is_empty() {
            writeln!(out, "stale: none")?;
        }
        for page in &self.stale {
            writeln!(
                out,
                "stale: {} ({} reason(s))",
                page.path,
                page.reasons.len()
            )?;
        }
        for page in &self.preexisting_broken {
            writeln!(
                out,
                "pre-existing broken anchor: {} ({} anchor(s); not touched by this feature, no verdict needed)",
                page.path,
                page.reasons.len()
            )?;
        }
        for warning in &self.warnings {
            writeln!(out, "warning: {warning}")?;
        }
        match &self.last {
            Some(last) => writeln!(out, "last: {} at {}", last.source_event, last.occurred_at),
            None => writeln!(out, "last: never reviewed"),
        }
    }
}

/// Whether the feature's sessions ran a `memory_query`, or `None` when
/// memory cannot tell: no database, or rows that could not be read.
#[must_use]
pub fn analysis_queried(layout: &Layout, name: &FeatureName) -> Option<bool> {
    let _ = drain::try_drain(layout);
    if !layout.memory_db().exists() {
        return None;
    }
    let feature = relations::read_feature(layout, name).ok()?;
    let mut notes = Notes::default();
    let rows = observations(layout, &feature, Access::Drain, &mut notes).ok()?;
    let mut facts: Vec<ObservationFact> = rows.iter().map(fact).collect();
    facts.extend(usage_facts(
        layout,
        &feature,
        &rows,
        Access::Drain,
        &mut notes,
    ));
    (!notes.incomplete).then(|| queried_before_approval(&facts, name.as_str()))
}

/// # Errors
///
/// When the feature does not exist. Unreadable state is a warning with
/// empty or partial lists, never an error.
pub fn review_in(layout: &Layout, name: &FeatureName) -> Result<Report<ReviewReport>, Failure> {
    review_with(layout, name, Access::Drain)
}

/// [`review_in`] for a session that cannot write `.ivar/`: the same report,
/// read without draining the capture spool, taking `memory.lock`, migrating or
/// creating anything. Observations still in the spool are not counted, and a
/// warning says so.
///
/// # Errors
///
/// As [`review_in`]; also when `memory.db` is from an older ivar (a read never
/// migrates it), a newer one, or not a memory database.
pub fn review_read_only_in(
    layout: &Layout,
    name: &FeatureName,
) -> Result<Report<ReviewReport>, Failure> {
    review_with(layout, name, Access::ReadOnly)
}

/// [`review_read_only_in`] for the hall around `ctx`.
///
/// # Errors
///
/// Outside a hall, for an unknown feature, or as [`review_read_only_in`].
pub fn review_read_only(ctx: &Ctx, feature: Option<String>) -> Outcome<ReviewReport> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(resolve_single_feature(
        ctx,
        feature,
        "Select a feature to review",
    )?)?;
    review_read_only_in(&layout, &name)
}

/// How a review reaches `memory.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Drains the spool first and opens the database for writing, which
    /// migrates it.
    Drain,
    /// Writes nothing under `.ivar/`.
    ReadOnly,
}

fn review_with(
    layout: &Layout,
    name: &FeatureName,
    access: Access,
) -> Result<Report<ReviewReport>, Failure> {
    let feature = relations::read_feature(layout, name)?;
    let mut notes = Notes::default();
    match access {
        Access::Drain => {
            if let Err(failure) = drain::drain(layout) {
                notes.warn(format!(
                    "the capture spool could not be drained: {}",
                    failure.what
                ));
            }
        }
        Access::ReadOnly => {
            refuse_unreadable_schema(layout)?;
            let pending = SpoolStatus::read(&layout.memory_spool_dir());
            notes.warn(format!(
                "the capture spool was not drained (read-only): {} file(s), {} bytes still pending and not counted",
                pending.files, pending.bytes
            ));
        }
    }
    let rows = observations(layout, &feature, access, &mut notes)?;
    let mut facts: Vec<ObservationFact> = rows.iter().map(fact).collect();
    facts.extend(usage_facts(layout, &feature, &rows, access, &mut notes));
    let written = written_pages(&facts, layout.root().as_str());
    let analysis_approved =
        crate::action::plan::effective_gate(layout, name, Gate::Analysis)? == GateState::Approved;
    let mut gaps = gaps(&facts, &written, name.as_str(), analysis_approved);
    let (unattributed, queries_before_approval) = unattributed_since_start(
        layout, &feature, &rows, &facts, &written, access, &mut notes,
    );
    explain_gaps(&mut gaps, &unattributed, queries_before_approval);
    let changed = changed_files(layout, &feature, &mut notes);
    let (anchored, broken) = anchor_matches(layout, &changed, access, &mut notes);
    let staleness = stale_pages(&anchored, &changed, &broken, &written);
    Ok(Report::new(ReviewReport {
        feature: name.to_string(),
        written,
        unattributed,
        gaps,
        stale: staleness.stale,
        preexisting_broken: staleness.preexisting,
        last: last_record(&rows, name),
        warnings: notes.warnings,
        incomplete: notes.incomplete,
    }))
}

/// A read-only review of a database it can never read as a memory database
/// fails instead of reporting an empty memory: older (a read never migrates
/// it), newer, or another program's.
fn refuse_unreadable_schema(layout: &Layout) -> Result<(), Failure> {
    if !layout.memory_db().exists() {
        return Ok(());
    }
    match open_current_read_only(layout.memory_db().as_std_path()) {
        Ok(ReadOnlyOpen::Outdated { found, current }) => Err(outdated(found, current)),
        Err(
            error @ (MemoryDbError::ForeignApplication { .. } | MemoryDbError::NewerSchema { .. }),
        ) => Err(Failure::failed(
            "memory.review_unreadable",
            format!("could not read `{}`: {error}", layout.memory_db()),
        )),
        Ok(ReadOnlyOpen::Current(_)) | Err(MemoryDbError::Sqlite(_)) => Ok(()),
    }
}

/// Why a read-only command cannot use a `memory.db` an older ivar wrote.
pub(crate) fn outdated(found: i64, current: i64) -> Failure {
    Failure::blocked(
        "memory.read_only_outdated",
        format!(
            "memory.db has schema {found} and this ivar reads schema {current}; \
             `--read-only` never migrates it, so run `ivar memory status` from a session that can write `.ivar/`"
        ),
    )
    .fix(
        FixAction::safe(
            "memory.migrate",
            "Migrate memory.db from a session that can write `.ivar/`, then run the command again.",
        )
        .command("ivar memory status"),
    )
}

/// # Errors
///
/// `memory.review_missing_verdicts` when a stale page has no verdict;
/// `memory.review_not_recorded` when the observation cannot be spooled;
/// `memory.review_unreadable` when memory, the manifest, a diff or the anchors
/// cannot be read, so nothing is attested.
pub fn confirm_in(
    layout: &Layout,
    scope: &MemoryEventScope,
    name: &FeatureName,
    verdicts: &[(String, Verdict)],
    not_required: Option<String>,
) -> Result<Report<ReviewReport>, Failure> {
    let review = review_in(layout, name)?.value;
    if review.incomplete {
        return Err(Failure::failed(
            "memory.review_unreadable",
            format!(
                "the review of `{name}` is incomplete ({}), so there is no review to confirm",
                review.warnings.join("; ")
            ),
        )
        .fix(FixAction::safe(
            "memory.inspect_db",
            "Fix the causes listed above (memory database, manifest, a repo's diff or the graph), then review again.",
        )));
    }
    if not_required.is_some() && !review.written.is_empty() {
        let count = review.written.len();
        return Err(Failure::blocked(
            "memory.review_not_required_with_pages",
            format!(
                "`not_required` is for a feature that wrote no page, and `{name}` wrote {count}"
            ),
        )
        .fix(FixAction::safe(
            "memory.review_drop_not_required",
            format!("Drop `not_required`: the feature wrote {count} page(s)."),
        )));
    }
    let missing = missing_verdicts(&review.stale, verdicts);
    if !missing.is_empty() {
        return Err(Failure::blocked(
            "memory.review_missing_verdicts",
            format!("every possibly stale page needs a verdict; missing: {}", missing.join(", ")),
        )
        .fix(FixAction::safe(
            "memory.review_give_verdicts",
            "Confirm again with `stale: [{path, verdict}]`, `updated` or `still_valid`, for each listed page.",
        )));
    }
    let feature = relations::read_feature(layout, name)?;
    let body = json!({
        "heads": feature_heads(layout, &feature),
        "verdicts": verdicts.iter().take(MAX_RECORDED_ENTRIES).collect::<Vec<_>>(),
        "not_required": Value::from(not_required),
        "written": review.written.iter().take(MAX_RECORDED_ENTRIES).map(|page| &page.path).collect::<Vec<_>>(),
        "gaps": review.gaps.iter().map(|gap| gap.kind).collect::<Vec<_>>(),
        "stale": review.stale.iter().take(MAX_RECORDED_ENTRIES).map(|page| &page.path).collect::<Vec<_>>(),
    });
    let scope = MemoryEventScope {
        feature: name.to_string(),
        ..scope.clone()
    };
    record_memory_event(layout, &scope, SourceEvent::MemoryReviewed, &body)
        .map_err(|reason| Failure::failed("memory.review_not_recorded", reason))?;
    review_in(layout, name)
}

/// What a feature's observations say about one of its pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageStanding {
    /// The feature wrote the page and has not deleted it.
    pub owned: bool,
    /// The feature's delivery has begun.
    pub delivered: bool,
}

/// How `name` stands with `rel`: whether it wrote the page and whether its
/// delivery has begun. Read from the feature's own observations
/// after draining the spool, so a page written a moment ago is seen. The
/// caller must hold `memory.lock`.
///
/// # Errors
///
/// When the spool cannot be drained or `memory.db` cannot be read.
pub(crate) fn owns_page_locked(
    layout: &Layout,
    name: &FeatureName,
    rel: &str,
) -> Result<PageStanding, String> {
    drain::drain_locked(layout, usize::MAX).map_err(|failure| failure.what)?;
    if !layout.memory_db().exists() {
        return Ok(PageStanding {
            owned: false,
            delivered: false,
        });
    }
    let own = name.to_string();
    let rows = open(layout)
        .and_then(|db| {
            feature_observations(&db, std::slice::from_ref(&own))
                .map_err(|error| unreadable(&error))
        })
        .map_err(|failure| failure.what)?;
    let facts: Vec<ObservationFact> = rows
        .iter()
        .filter(|row| row.feature == own)
        .map(fact)
        .collect();
    let owned = written_pages(&facts, layout.root().as_str())
        .iter()
        .any(|page| page.path == rel && page.action != PageAction::Deleted);
    Ok(PageStanding {
        owned,
        delivered: is_delivered(&facts),
    })
}

/// # Errors
///
/// When `memory.db` exists but cannot be read.
pub fn review_state(layout: &Layout, name: &FeatureName) -> Result<ReviewState, Failure> {
    if !layout.memory_db().exists() {
        return Ok(ReviewState::Missing);
    }
    let db = open(layout)?;
    let rows =
        feature_observations(&db, &[name.to_string()]).map_err(|error| unreadable(&error))?;
    Ok(match last_record(&rows, name) {
        Some(record) if record.source_event == SourceEvent::MemoryReviewed.as_str() => {
            ReviewState::Reviewed {
                heads: record.heads,
            }
        }
        Some(_) => ReviewState::Skipped,
        None => ReviewState::Missing,
    })
}

#[must_use]
pub fn feature_heads(layout: &Layout, feature: &Feature) -> BTreeMap<String, String> {
    let git = git::System;
    feature
        .promotions
        .iter()
        .filter(|(_, promotion)| promotion.worktree == WorktreeState::Ready)
        .filter_map(|(repo, _)| {
            git.head_commit(&layout.repo_worktree(repo, &feature.branch))
                .ok()
                .map(|sha| (repo.to_string(), sha))
        })
        .collect()
}

#[derive(Default)]
struct Notes {
    warnings: Vec<String>,
    incomplete: bool,
}

impl Notes {
    fn warn(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    fn unread(&mut self, warning: String) {
        self.incomplete = true;
        self.warnings.push(warning);
    }
}

fn open(layout: &Layout) -> Result<MemoryDb, Failure> {
    MemoryDb::open(layout.memory_db().as_std_path())
        .map_err(|error| Failure::failed("memory.review_unreadable", error.to_string()))
}

/// The connection a review reads through: the writable database, or for
/// [`Access::ReadOnly`] one that never creates, migrates or writes a file.
fn open_conn(layout: &Layout, access: Access) -> Result<Connection, Failure> {
    match access {
        Access::Drain => open(layout).map(MemoryDb::into_conn),
        Access::ReadOnly => match open_current_read_only(layout.memory_db().as_std_path()) {
            Ok(ReadOnlyOpen::Current(conn)) => Ok(conn),
            Ok(ReadOnlyOpen::Outdated { found, current }) => Err(outdated(found, current)),
            Err(error) => Err(Failure::failed(
                "memory.review_unreadable",
                error.to_string(),
            )),
        },
    }
}

fn unreadable(error: &rusqlite::Error) -> Failure {
    Failure::failed("memory.review_unreadable", error.to_string())
}

fn observations(
    layout: &Layout,
    feature: &Feature,
    access: Access,
    notes: &mut Notes,
) -> Result<Vec<FeatureObservation>, Failure> {
    if !layout.memory_db().exists() {
        notes.warn("memory.db does not exist yet: nothing was captured".to_owned());
        return Ok(Vec::new());
    }
    let mut names = vec![feature.name.to_string()];
    names.extend(
        relations::descendants(layout, &feature.name)?
            .into_iter()
            .map(|child| child.name.to_string()),
    );
    let rows = open_conn(layout, access).and_then(|conn| {
        feature_observations_on(&conn, &names).map_err(|error| unreadable(&error))
    });
    Ok(rows.unwrap_or_else(|failure| {
        notes.unread(format!("memory could not be read: {}", failure.what));
        Vec::new()
    }))
}

fn fact(row: &FeatureObservation) -> ObservationFact {
    ObservationFact {
        source_event: row.source_event.clone(),
        occurred_at: row.occurred_at.clone(),
        importance: row.importance,
        body: serde_json::from_str(&row.body).unwrap_or_else(|_| Value::String(row.body.clone())),
    }
}

/// The `memory_usage` rows of the feature's sessions, as facts: a query served
/// through MCP or the CLI by a client no hook captures is recorded only there.
/// A query an unbound MCP server attributed to the feature by argument has the
/// feature's stand-in session.
fn usage_facts(
    layout: &Layout,
    feature: &Feature,
    rows: &[FeatureObservation],
    access: Access,
    notes: &mut Notes,
) -> Vec<ObservationFact> {
    let attributed = std::iter::once(feature.name.clone())
        .chain(
            relations::descendants(layout, &feature.name)
                .unwrap_or_default()
                .into_iter()
                .map(|child| child.name),
        )
        .map(|name| feature_usage_session(&name));
    let sessions: Vec<String> = rows
        .iter()
        .filter(|row| !row.session_id.is_empty())
        .map(|row| row.session_id.clone())
        .chain(attributed)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if sessions.is_empty() {
        return Vec::new();
    }
    let queries = open_conn(layout, access)
        .and_then(|conn| session_queries(&conn, &sessions).map_err(|error| unreadable(&error)));
    match queries {
        Ok(queries) => queries
            .into_iter()
            .map(|ts| ObservationFact {
                source_event: USAGE_EVENT.to_owned(),
                occurred_at: rfc3339_from_epoch_secs(ts as f64),
                importance: 0,
                body: Value::Null,
            })
            .collect(),
        Err(failure) => {
            notes.warn(format!("memory usage could not be read: {}", failure.what));
            Vec::new()
        }
    }
}

/// The epoch second the feature's window opens at: the earlier of its
/// directory's creation and its first observation. A feature records no
/// creation time of its own.
fn window_start(layout: &Layout, feature: &Feature, rows: &[FeatureObservation]) -> Option<i64> {
    let created = std::fs::metadata(layout.feature_dir(&feature.name))
        .and_then(|meta| meta.created())
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok());
    let first = rows
        .iter()
        .filter(|row| row.feature == feature.name.as_str())
        .filter_map(|row| epoch_secs_from_rfc3339(&row.occurred_at))
        .min();
    created.into_iter().chain(first).min()
}

/// The writes and queries no session or feature claimed since the feature
/// began, and how many of those queries preceded the Analysis approval (the
/// whole window while no approval was captured). They never reach `written`
/// or the gaps' facts: a call of another feature in the hall can fall inside
/// the window.
fn unattributed_since_start(
    layout: &Layout,
    feature: &Feature,
    rows: &[FeatureObservation],
    facts: &[ObservationFact],
    written: &[WrittenPage],
    access: Access,
    notes: &mut Notes,
) -> (Unattributed, usize) {
    let none = (Unattributed::default(), 0);
    let Some(start) = window_start(layout, feature, rows) else {
        return none;
    };
    if !layout.memory_db().exists() {
        return none;
    }
    let since = rfc3339_from_epoch_secs(start as f64);
    let read = open_conn(layout, access).and_then(|conn| {
        let writes = unattributed_page_writes(&conn, &since).map_err(|error| unreadable(&error))?;
        let queries = unattributed_queries(&conn, start).map_err(|error| unreadable(&error))?;
        Ok((writes, queries))
    });
    let (writes, queries) = match read {
        Ok(read) => read,
        Err(failure) => {
            notes.warn(format!(
                "unattributed memory calls could not be read: {}",
                failure.what
            ));
            return none;
        }
    };
    let write_facts: Vec<ObservationFact> = writes.iter().map(fact).collect();
    let attributed: std::collections::BTreeSet<&str> =
        written.iter().map(|page| page.path.as_str()).collect();
    let pages = written_pages(&write_facts, layout.root().as_str())
        .into_iter()
        .filter(|page| !attributed.contains(page.path.as_str()))
        .collect();
    let before_approval = match analysis_approved_at(facts, feature.name.as_str()) {
        Some(approved_at) => queries
            .iter()
            .filter(|ts| rfc3339_from_epoch_secs(**ts as f64).as_str() < approved_at)
            .count(),
        None => queries.len(),
    };
    (
        Unattributed {
            written: pages,
            queries: queries.len(),
        },
        before_approval,
    )
}

/// The latest `memory.reviewed` of the feature, else its latest
/// `memory.review_skipped`: a review outranks any skip.
fn last_record(rows: &[FeatureObservation], name: &FeatureName) -> Option<ReviewRecord> {
    let latest = |event: SourceEvent| {
        rows.iter()
            .rev()
            .find(|row| row.feature == name.as_str() && row.source_event == event.as_str())
    };
    let row =
        latest(SourceEvent::MemoryReviewed).or_else(|| latest(SourceEvent::MemoryReviewSkipped))?;
    let body: Value = serde_json::from_str(&row.body).unwrap_or_else(|_| cut_body_heads(&row.body));
    Some(ReviewRecord {
        source_event: row.source_event.clone(),
        occurred_at: row.occurred_at.clone(),
        verdicts: field(&body, "verdicts"),
        not_required: field(&body, "not_required"),
        reason: field(&body, "reason"),
        heads: field(&body, "heads"),
        written: field(&body, "written"),
        stale: field(&body, "stale"),
    })
}

/// A body cut by the sanitize limit is no longer JSON. serde_json writes keys
/// sorted, so `heads` follows only the short `gaps` list and survives the cut.
fn cut_body_heads(raw: &str) -> Value {
    const KEY: &str = "\"heads\":";
    let Some(start) = raw.find(KEY).map(|at| at + KEY.len()) else {
        return Value::Null;
    };
    let object = &raw[start..];
    object
        .find('}')
        .and_then(|end| serde_json::from_str::<Value>(&object[..=end]).ok())
        .map_or(Value::Null, |heads| json!({ "heads": heads }))
}

fn field<T: serde::de::DeserializeOwned + Default>(body: &Value, key: &str) -> T {
    body.get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn changed_files(layout: &Layout, feature: &Feature, notes: &mut Notes) -> Vec<(String, String)> {
    let manifest = match read_manifest(layout) {
        Ok(manifest) => manifest,
        Err(failure) => {
            notes.unread(format!("the manifest could not be read: {}", failure.what));
            return Vec::new();
        }
    };
    let git = git::System;
    let mut files = Vec::new();
    for (repo, promotion) in &feature.promotions {
        if promotion.worktree != WorktreeState::Ready {
            continue;
        }
        let Some(declared) = manifest
            .repos()
            .iter()
            .find(|declared| declared.name() == repo)
        else {
            continue;
        };
        let base = base::resolve(feature, promotion, declared.default_branch());
        let diff = git
            .merge_base(
                &layout.repo_bare(repo),
                base.as_str(),
                feature.branch.as_str(),
            )
            .and_then(|merge_base| {
                git.diff_worktree_files(
                    &layout.repo_worktree(repo, &feature.branch),
                    Some(merge_base.as_str()),
                )
            });
        match diff {
            Ok(diff) => files.extend(
                diff.modified_or_added
                    .into_iter()
                    .chain(diff.deleted)
                    .map(|path| (repo.to_string(), path.to_string())),
            ),
            Err(error) => notes.unread(format!("the diff of `{repo}` could not be read: {error}")),
        }
    }
    files
}

type Anchored = Vec<(String, String, String)>;
type Broken = Vec<(String, String, String, Option<String>)>;

/// Every broken anchor of a durable page; [`stale_pages`] keeps those whose
/// file the diff touched and reports the rest as pre-existing.
fn anchor_matches(
    layout: &Layout,
    changed: &[(String, String)],
    access: Access,
    notes: &mut Notes,
) -> (Anchored, Broken) {
    let conn = match open_conn(layout, access) {
        Ok(conn) => conn,
        Err(failure) => {
            notes.unread(format!("the anchors could not be read: {}", failure.what));
            return (Vec::new(), Vec::new());
        }
    };
    let anchored = anchored_files(&conn, changed).unwrap_or_else(|error| {
        notes.unread(format!("the anchors could not be read: {error}"));
        Vec::new()
    });
    let graph = match access {
        Access::Drain => attach_graph(&conn, &layout.graph_db()),
        Access::ReadOnly => attach_graph_untouched(&conn, &layout.graph_db()),
    };
    match graph {
        GraphAttach::Attached => {}
        GraphAttach::Missing => {
            notes.warn("graph.db is missing: broken anchors were not checked".to_owned());
            return (anchored, Vec::new());
        }
        GraphAttach::Unreadable => {
            notes.unread("graph.db could not be read: broken anchors were not checked".to_owned());
            return (anchored, Vec::new());
        }
    }
    let broken = broken_anchors(&conn)
        .map(|rows| {
            rows.into_iter()
                .filter(|(page, _)| is_durable_page(page))
                .map(|(page, anchor)| (page, anchor.repo, anchor.path, anchor.symbol))
                .collect()
        })
        .unwrap_or_else(|error| {
            notes.unread(format!("broken anchors could not be read: {error}"));
            Vec::new()
        });
    detach_graph(&conn);
    (anchored, broken)
}
