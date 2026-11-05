//! `ivar memory query`: pages by reciprocal-rank fusion, observations as fallback.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::action::memory::reindex;
use crate::action::{Ctx, discover_hall};
use crate::domain::memory_page::{PageKind, Tier};
use crate::domain::memory_rank::{
    NEIGHBOUR_SEEDS, PINNED_BONUS, fts_expression, rrf_contribution, tier_multiplier,
};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observation_search;
use crate::store::memory::pages::{self, PageFilter, PageHit};

const MAX_LIMIT: usize = 20;
const CANDIDATES: usize = 50;

#[derive(Debug, Clone)]
pub struct QueryInput {
    pub text: String,
    pub kinds: Vec<String>,
    pub tier: Option<String>,
    pub tag: Option<String>,
    pub limit: usize,
    pub include_expired: bool,
    pub explain: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRank {
    pub source: String,
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryHit {
    pub path: String,
    pub kind: String,
    pub tier: String,
    pub title: String,
    pub snippet: String,
    pub pinned: bool,
    pub score: f64,
    pub sources: Vec<SourceRank>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationHit {
    pub source_event: String,
    pub occurred_at: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryOutcome {
    pub pages: Vec<QueryHit>,
    pub observations: Vec<ObservationHit>,
}

impl WriteHuman for QueryOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        for hit in &self.pages {
            writeln!(
                out,
                "{} [{}/{}] {}",
                hit.path, hit.kind, hit.tier, hit.title
            )?;
            writeln!(out, "  {}", hit.snippet)?;
        }
        for hit in &self.observations {
            writeln!(
                out,
                "{} {} {}",
                hit.occurred_at, hit.source_event, hit.snippet
            )?;
        }
        Ok(())
    }
}

struct Ranked {
    hit: PageHit,
    score: f64,
    sources: Vec<SourceRank>,
}

fn sqlite_failure(error: &rusqlite::Error) -> Failure {
    Failure::failed(
        "memory.query_failed",
        format!("could not search the memory pages: {error}"),
    )
}

fn parse_kinds(kinds: &[String]) -> Result<Vec<PageKind>, Failure> {
    kinds
        .iter()
        .map(|kind| {
            PageKind::parse(kind).ok_or_else(|| {
                Failure::failed("memory.kind_unknown", format!("unknown page kind `{kind}`"))
            })
        })
        .collect()
}

fn parse_tier(tier: Option<&str>) -> Result<Option<Tier>, Failure> {
    tier.map(|tier| {
        Tier::parse(tier).ok_or_else(|| {
            Failure::failed("memory.tier_unknown", format!("unknown page tier `{tier}`"))
        })
    })
    .transpose()
}

fn fuse(
    db: &MemoryDb,
    expression: &str,
    filter: &PageFilter,
    filtered: bool,
) -> rusqlite::Result<Vec<Ranked>> {
    let fts_hits = pages::search_pages(db, expression, filter, CANDIDATES)?;
    let mut ranked: BTreeMap<String, Ranked> = BTreeMap::new();
    for (rank, hit) in fts_hits.iter().enumerate() {
        ranked.insert(
            hit.path.clone(),
            Ranked {
                hit: hit.clone(),
                score: rrf_contribution(rank),
                sources: vec![SourceRank {
                    source: "fts".into(),
                    rank,
                }],
            },
        );
    }
    if filtered {
        return Ok(ranked.into_values().collect());
    }
    let seeds: Vec<String> = fts_hits
        .iter()
        .take(NEIGHBOUR_SEEDS)
        .map(|hit| hit.path.clone())
        .collect();
    for (index, path) in pages::linked_paths(db, &seeds)?.into_iter().enumerate() {
        let rank = NEIGHBOUR_SEEDS + index;
        if let Some(entry) = ranked.get_mut(&path) {
            entry.score += rrf_contribution(rank);
            entry.sources.push(SourceRank {
                source: "links".into(),
                rank,
            });
        } else if let Some(mut hit) = pages::page_by_path(db, &path, None)? {
            hit.snippet.clone_from(&hit.title);
            ranked.insert(
                path,
                Ranked {
                    hit,
                    score: rrf_contribution(rank),
                    sources: vec![SourceRank {
                        source: "links".into(),
                        rank,
                    }],
                },
            );
        }
    }
    Ok(ranked.into_values().collect())
}

fn into_query_hit(ranked: Ranked, explain: bool) -> QueryHit {
    let Ranked {
        hit,
        score,
        sources,
    } = ranked;
    let pinned_bonus = if hit.pinned { PINNED_BONUS } else { 1.0 };
    QueryHit {
        score: score * tier_multiplier(hit.tier) * pinned_bonus,
        path: hit.path,
        kind: hit.kind.as_str().to_owned(),
        tier: hit.tier.as_str().to_owned(),
        title: hit.title,
        snippet: hit.snippet,
        pinned: hit.pinned,
        sources: if explain { sources } else { Vec::new() },
    }
}

/// # Errors
///
/// When no hall is found, a kind or tier is unknown, or the index cannot be read.
pub fn query(ctx: &Ctx, input: &QueryInput) -> Outcome<QueryOutcome> {
    let layout = discover_hall(ctx)?;
    let kinds = parse_kinds(&input.kinds)?;
    let tier = parse_tier(input.tier.as_deref())?;
    let limit = input.limit.clamp(1, MAX_LIMIT);
    let filtered = !kinds.is_empty() || tier.is_some() || input.tag.is_some();
    let filter = PageFilter {
        kinds,
        tier,
        tag: input.tag.clone(),
        include_expired: input.include_expired,
        now: rfc3339_now(),
    };
    reindex::refresh(&layout)?;
    let Some(expression) = fts_expression(&input.text) else {
        return Ok(Report::new(QueryOutcome {
            pages: Vec::new(),
            observations: Vec::new(),
        }));
    };
    let db = MemoryDb::open(layout.memory_db().as_std_path())
        .map_err(|error| Failure::failed("memory.db_unavailable", error.to_string()))?;
    let mut pages: Vec<QueryHit> = fuse(&db, &expression, &filter, filtered)
        .map_err(|error| sqlite_failure(&error))?
        .into_iter()
        .map(|ranked| into_query_hit(ranked, input.explain))
        .collect();
    pages.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
    });
    pages.truncate(limit);
    let observations = if pages.is_empty() && !filtered {
        observation_search::search(&db, &expression, limit)
            .map_err(|error| sqlite_failure(&error))?
            .into_iter()
            .map(|found| ObservationHit {
                source_event: found.source_event,
                occurred_at: found.occurred_at,
                snippet: found.snippet,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Report::new(QueryOutcome {
        pages,
        observations,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/query.rs"]
mod tests;
