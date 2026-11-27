//! `ivar memory eval`: measure retrieval quality against hand-labelled queries.
//!
//! `sample` exports queries from the usage log; a human fills in `expected`.
//! `run` ranks each labelled query with the same `search` as `ivar memory
//! query` and scores it. It never records usage.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::memory::drain::epoch_secs_now;
use crate::action::memory::query::{MAX_LIMIT, QueryInput, search};
use crate::action::{Ctx, discover_hall};
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::store::memory::db::{MemoryDb, ReadOnlyOpen, open_current_read_only};
use crate::store::memory::pages;

/// One line of the label file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub query: String,
    #[serde(default)]
    pub expected: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleOutcome {
    pub out: Option<PathBuf>,
    pub queries: Vec<Label>,
}

impl WriteHuman for SampleOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        match &self.out {
            Some(path) => writeln!(
                w,
                "wrote {} queries to {}",
                self.queries.len(),
                path.display()
            ),
            None => {
                for label in &self.queries {
                    writeln!(w, "{}", jsonl(label))?;
                }
                Ok(())
            }
        }
    }
}

fn jsonl(label: &Label) -> String {
    serde_json::to_string(label).unwrap_or_default()
}

/// # Errors
///
/// When no hall or memory database is found, it cannot be read, or `out` cannot be written.
pub fn sample(ctx: &Ctx, days: u64, limit: usize, out: Option<PathBuf>) -> Outcome<SampleOutcome> {
    let layout = discover_hall(ctx)?;
    let path = layout.memory_db();
    let conn = match open_current_read_only(path.as_std_path()) {
        Ok(ReadOnlyOpen::Current(conn)) => conn,
        Ok(ReadOnlyOpen::Outdated { found, current }) => {
            return Err(crate::action::memory::review::outdated(found, current));
        }
        Err(error) => {
            return Err(Failure::failed(
                "memory.eval_unreadable",
                format!("could not read `{path}`: {error}"),
            ));
        }
    };
    let since = i64::try_from(epoch_secs_now().saturating_sub(days.saturating_mul(86_400)))
        .unwrap_or(i64::MAX);
    let read_failed = |e: rusqlite::Error| Failure::failed("memory.read_failed", e.to_string());
    let mut stmt = conn
        .prepare(
            "SELECT query FROM memory_usage
              WHERE source != 'hook' AND ts >= ?1 AND query IS NOT NULL AND trim(query) != ''
              GROUP BY query ORDER BY max(ts) DESC, query LIMIT ?2",
        )
        .map_err(read_failed)?;
    let queries = stmt
        .query_map(
            rusqlite::params![since, i64::try_from(limit).unwrap_or(i64::MAX)],
            |row| row.get::<_, String>(0),
        )
        .map_err(read_failed)?
        .map(|row| {
            row.map(|query| Label {
                query,
                expected: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(read_failed)?;
    if let Some(file) = &out {
        let text: String = queries.iter().map(|l| jsonl(l) + "\n").collect();
        std::fs::File::create(file)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| {
                Failure::failed(
                    "memory.eval_write_failed",
                    format!("could not write `{}`: {e}", file.display()),
                )
            })?;
    }
    Ok(Report::new(SampleOutcome { out, queries }))
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryScore {
    pub query: String,
    pub expected: Vec<String>,
    /// Expected pages that no longer exist; left out of the scores.
    pub missing: Vec<String>,
    pub returned: Vec<String>,
    pub hit: bool,
    pub recall: f64,
    pub reciprocal_rank: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aggregate {
    pub hit_rate: f64,
    pub recall: f64,
    pub mrr: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalOutcome {
    pub k: usize,
    pub scored: usize,
    /// Lines with an empty `expected`.
    pub skipped_unlabelled: usize,
    /// Lines whose expected pages all no longer exist.
    pub skipped_unresolvable: usize,
    pub aggregate: Aggregate,
    pub queries: Vec<QueryScore>,
}

impl WriteHuman for EvalOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        let k = self.k;
        for q in &self.queries {
            writeln!(
                w,
                "{} hit@{k}={} recall@{k}={:.3} rr={:.3}",
                q.query,
                u8::from(q.hit),
                q.recall,
                q.reciprocal_rank
            )?;
        }
        writeln!(
            w,
            "scored {} queries (skipped {} unlabelled, {} unresolvable)",
            self.scored, self.skipped_unlabelled, self.skipped_unresolvable
        )?;
        writeln!(
            w,
            "hit@{k}={:.3} recall@{k}={:.3} MRR={:.3}",
            self.aggregate.hit_rate, self.aggregate.recall, self.aggregate.mrr
        )
    }
}

fn read_labels(file: &Path) -> Result<Vec<Label>, Failure> {
    let text = std::fs::read_to_string(file).map_err(|e| {
        Failure::failed(
            "memory.eval_unreadable",
            format!("could not read `{}`: {e}", file.display()),
        )
    })?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<Label>(line).map_err(|e| {
                Failure::failed(
                    "memory.eval_bad_label",
                    format!("{}:{}: {e}", file.display(), index + 1),
                )
            })
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn mean(values: impl Iterator<Item = f64>, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        values.sum::<f64>() / count as f64
    }
}

/// # Errors
///
/// When no hall is found, the label file is unreadable or malformed, or the index cannot be read.
#[allow(clippy::cast_precision_loss)]
pub fn run(ctx: &Ctx, file: &Path, k: usize) -> Outcome<EvalOutcome> {
    let layout = discover_hall(ctx)?;
    let k = k.clamp(1, MAX_LIMIT);
    let labels = read_labels(file)?;
    let mut warnings = Vec::new();
    let mut scores = Vec::new();
    let (mut unlabelled, mut unresolvable) = (0, 0);
    for label in labels {
        if label.expected.is_empty() {
            unlabelled += 1;
            continue;
        }
        let returned: Vec<String> = search(
            &layout,
            &QueryInput {
                text: label.query.clone(),
                kinds: Vec::new(),
                tier: None,
                tag: None,
                limit: MAX_LIMIT,
                include_expired: false,
                explain: false,
            },
        )?
        .value
        .pages
        .into_iter()
        .map(|hit| hit.path)
        .collect();
        let db = MemoryDb::open(layout.memory_db().as_std_path())
            .map_err(|e| Failure::failed("memory.db_unavailable", e.to_string()))?;
        let mut live = BTreeSet::new();
        let mut missing = Vec::new();
        for path in &label.expected {
            let found = pages::page_by_path(&db, path, None, None)
                .map_err(|e| Failure::failed("memory.read_failed", e.to_string()))?;
            if found.is_some() {
                live.insert(path.clone());
            } else {
                warnings.push(Warning::new(
                    "memory.eval_page_missing",
                    label.query.clone(),
                    format!("expected page `{path}` does not exist"),
                ));
                missing.push(path.clone());
            }
        }
        if live.is_empty() {
            unresolvable += 1;
            continue;
        }
        let in_top = returned
            .iter()
            .take(k)
            .filter(|p| live.contains(*p))
            .count();
        let first = returned.iter().position(|p| live.contains(p));
        scores.push(QueryScore {
            query: label.query,
            expected: label.expected,
            missing,
            returned: returned.into_iter().take(k).collect(),
            hit: in_top > 0,
            recall: in_top as f64 / live.len() as f64,
            reciprocal_rank: first.map_or(0.0, |i| 1.0 / (i + 1) as f64),
        });
    }
    let n = scores.len();
    let aggregate = Aggregate {
        hit_rate: mean(scores.iter().map(|s| f64::from(u8::from(s.hit))), n),
        recall: mean(scores.iter().map(|s| s.recall), n),
        mrr: mean(scores.iter().map(|s| s.reciprocal_rank), n),
    };
    Ok(Report::with_warnings(
        EvalOutcome {
            k,
            scored: n,
            skipped_unlabelled: unlabelled,
            skipped_unresolvable: unresolvable,
            aggregate,
            queries: scores,
        },
        warnings,
    ))
}
