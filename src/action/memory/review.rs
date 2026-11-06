//! `memory_review` and `ivar memory review`: what a feature wrote to memory,
//! its gaps and the durable pages its diff may have made stale, and the
//! confirmation that gates `deliver apply`.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use super::drain;
use super::events::{MemoryEventScope, record_memory_event};
use crate::action::feature::select::resolve_single_feature;
use crate::action::feature::{base, relations};
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::feature::{Feature, Gate, GateState, WorktreeState};
use crate::domain::memory_review::{
    Gap, ObservationFact, StalePage, Verdict, WrittenPage, gaps, is_durable_page, missing_verdicts,
    stale_pages, written_pages,
};
use crate::domain::name::FeatureName;
use crate::domain::observation::SourceEvent;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::store::layout::Layout;
use crate::store::memory::anchors::{
    GraphAttach, anchored_files, attach_graph, broken_anchors, detach_graph,
};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::{FeatureObservation, feature_observations};

const MAX_RECORDED_ENTRIES: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct ReviewReport {
    pub feature: String,
    pub written: Vec<WrittenPage>,
    pub gaps: Vec<Gap>,
    pub stale: Vec<StalePage>,
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

/// # Errors
///
/// Outside a hall, for an unknown feature, or when memory cannot be read.
pub fn review(ctx: &Ctx, feature: Option<String>) -> Outcome<ReviewReport> {
    let layout = discover_hall(ctx)?;
    let name = resolve_single_feature(ctx, feature, "Select a feature to review")?;
    review_in(&layout, &FeatureName::new(name)?)
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
        for warning in &self.warnings {
            writeln!(out, "warning: {warning}")?;
        }
        match &self.last {
            Some(last) => writeln!(out, "last: {} at {}", last.source_event, last.occurred_at),
            None => writeln!(out, "last: never reviewed"),
        }
    }
}

/// # Errors
///
/// When the feature does not exist. Unreadable state is a warning with
/// empty or partial lists, never an error.
pub fn review_in(layout: &Layout, name: &FeatureName) -> Result<Report<ReviewReport>, Failure> {
    let feature = relations::read_feature(layout, name)?;
    let mut notes = Notes::default();
    if let Err(failure) = drain::drain(layout) {
        notes.warn(format!(
            "the capture spool could not be drained: {}",
            failure.what
        ));
    }
    let rows = observations(layout, &feature, &mut notes)?;
    let facts: Vec<ObservationFact> = rows.iter().map(fact).collect();
    let written = written_pages(&facts, layout.root().as_str());
    let analysis_approved =
        crate::action::plan::effective_gate(layout, name, Gate::Analysis)? == GateState::Approved;
    let gaps = gaps(&facts, &written, name.as_str(), analysis_approved);
    let changed = changed_files(layout, &feature, &mut notes);
    let (anchored, broken) = anchor_matches(layout, &changed, &mut notes);
    let stale = stale_pages(&anchored, &broken, &written);
    Ok(Report::new(ReviewReport {
        feature: name.to_string(),
        written,
        gaps,
        stale,
        last: last_record(&rows, name),
        warnings: notes.warnings,
        incomplete: notes.incomplete,
    }))
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

fn unreadable(error: &rusqlite::Error) -> Failure {
    Failure::failed("memory.review_unreadable", error.to_string())
}

fn observations(
    layout: &Layout,
    feature: &Feature,
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
    let rows = open(layout)
        .and_then(|db| feature_observations(&db, &names).map_err(|error| unreadable(&error)));
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

/// Every broken anchor of a durable page counts, wherever its file is.
fn anchor_matches(
    layout: &Layout,
    changed: &[(String, String)],
    notes: &mut Notes,
) -> (Anchored, Broken) {
    let db = match open(layout) {
        Ok(db) => db,
        Err(failure) => {
            notes.unread(format!("the anchors could not be read: {}", failure.what));
            return (Vec::new(), Vec::new());
        }
    };
    let anchored = anchored_files(db.conn(), changed).unwrap_or_else(|error| {
        notes.unread(format!("the anchors could not be read: {error}"));
        Vec::new()
    });
    match attach_graph(db.conn(), &layout.graph_db()) {
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
    let broken = broken_anchors(db.conn())
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
    detach_graph(db.conn());
    (anchored, broken)
}
