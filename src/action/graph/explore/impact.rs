use std::collections::HashSet;

use crate::action::graph::query::{self, SymbolLocation};
use crate::domain::graph::{
    CallFlowItem, ExploreImpact, OperationalRelation, RelationDirection, RelationEndpoint,
};
use crate::store::graph::db::GraphDb;

use super::error::ExploreError;

pub(crate) struct RelationsAnalysis {
    pub(crate) call_flows: Vec<CallFlowItem>,
    pub(crate) direct_relations: Vec<OperationalRelation>,
    pub(crate) entry_points: Vec<OperationalRelation>,
}

pub(crate) fn collect_relations(
    db: &GraphDb,
    candidates: &[SymbolLocation],
    repo: Option<&str>,
) -> Result<RelationsAnalysis, ExploreError> {
    let mut call_flows = Vec::new();
    let mut seen_flows = HashSet::new();
    let mut direct_relations = Vec::new();
    let mut entry_points = Vec::new();
    let mut seen_relations = HashSet::new();

    for candidate in candidates {
        let candidate_endpoint = RelationEndpoint {
            repo: candidate.symbol.repo.clone(),
            file_path: candidate.file_path.clone(),
            symbol_name: candidate.symbol.name.clone(),
            symbol_kind: Some(candidate.symbol.kind.clone()),
        };

        // Query callers (incoming)
        let callers = query::get_callers(db, &candidate.symbol.name, repo, true, 0.7)?;
        for caller in callers {
            let flow_key = (
                caller.caller.name.clone(),
                candidate.symbol.name.clone(),
                caller.line,
            );
            if seen_flows.insert(flow_key) {
                call_flows.push(CallFlowItem {
                    caller: caller.caller.name.clone(),
                    callee: candidate.symbol.name.clone(),
                    edge_kind: caller.edge_kind.clone(),
                    provenance: caller.provenance,
                    line: caller.line,
                });
            }

            let caller_endpoint = RelationEndpoint {
                repo: caller.caller.repo.clone(),
                file_path: caller.caller_file_path.clone(),
                symbol_name: caller.caller.name.clone(),
                symbol_kind: Some(caller.caller.kind.clone()),
            };
            let is_cross_repo = caller.caller.repo != candidate.symbol.repo;
            let rel_key = (
                caller_endpoint.repo.clone(),
                caller_endpoint.file_path.clone(),
                caller_endpoint.symbol_name.clone(),
                candidate_endpoint.repo.clone(),
                candidate_endpoint.file_path.clone(),
                candidate_endpoint.symbol_name.clone(),
                caller.line,
                caller.edge_kind.clone(),
            );
            if seen_relations.insert(rel_key) {
                let rel = OperationalRelation {
                    source: caller_endpoint.clone(),
                    target: candidate_endpoint.clone(),
                    direction: RelationDirection::Incoming,
                    edge_kind: caller.edge_kind,
                    provenance: caller.provenance,
                    confidence: caller.confidence,
                    line: caller.line,
                    hop_count: 1,
                    cross_repo: is_cross_repo,
                };
                // Check if this incoming caller serves as an entry point (e.g. exported or CLI/root caller)
                if caller.caller.is_exported
                    || caller.caller_file_path.contains("cli")
                    || caller.caller_file_path.contains("main")
                {
                    entry_points.push(rel.clone());
                }
                direct_relations.push(rel);
            }
        }

        // Query callees (outgoing)
        if let Some(sym_id) = candidate.symbol.id {
            let callees = query::get_callees(db, sym_id)?;
            for callee in callees {
                let flow_key = (
                    candidate.symbol.name.clone(),
                    callee.callee_name.clone(),
                    callee.line,
                );
                if seen_flows.insert(flow_key) {
                    call_flows.push(CallFlowItem {
                        caller: candidate.symbol.name.clone(),
                        callee: callee.callee_name.clone(),
                        edge_kind: callee.edge_kind.clone(),
                        provenance: callee.provenance,
                        line: callee.line,
                    });
                }

                let callee_repo = callee
                    .callee_symbol
                    .as_ref()
                    .map(|s| s.repo.clone())
                    .unwrap_or_else(|| candidate.symbol.repo.clone());
                let callee_file_path = callee.callee_file_path.clone().unwrap_or_default();
                let callee_kind = callee.callee_symbol.as_ref().map(|s| s.kind.clone());

                let target_endpoint = RelationEndpoint {
                    repo: callee_repo.clone(),
                    file_path: callee_file_path.clone(),
                    symbol_name: callee.callee_name.clone(),
                    symbol_kind: callee_kind,
                };
                let is_cross_repo = callee_repo != candidate.symbol.repo;
                let rel_key = (
                    candidate_endpoint.repo.clone(),
                    candidate_endpoint.file_path.clone(),
                    candidate_endpoint.symbol_name.clone(),
                    target_endpoint.repo.clone(),
                    target_endpoint.file_path.clone(),
                    target_endpoint.symbol_name.clone(),
                    callee.line,
                    callee.edge_kind.clone(),
                );
                if seen_relations.insert(rel_key) {
                    direct_relations.push(OperationalRelation {
                        source: candidate_endpoint.clone(),
                        target: target_endpoint,
                        direction: RelationDirection::Outgoing,
                        edge_kind: callee.edge_kind,
                        provenance: callee.provenance,
                        confidence: callee.confidence,
                        line: callee.line,
                        hop_count: 1,
                        cross_repo: is_cross_repo,
                    });
                }
            }
        }
    }

    // Deterministic sorting for direct relations, entry points, and call flows
    direct_relations.sort_by(|a, b| {
        a.cross_repo
            .cmp(&b.cross_repo)
            .then_with(|| a.source.repo.cmp(&b.source.repo))
            .then_with(|| a.source.file_path.cmp(&b.source.file_path))
            .then_with(|| a.source.symbol_name.cmp(&b.source.symbol_name))
            .then_with(|| a.target.symbol_name.cmp(&b.target.symbol_name))
            .then_with(|| a.line.cmp(&b.line))
    });

    entry_points.sort_by(|a, b| {
        a.source
            .repo
            .cmp(&b.source.repo)
            .then_with(|| a.source.file_path.cmp(&b.source.file_path))
            .then_with(|| a.source.symbol_name.cmp(&b.source.symbol_name))
            .then_with(|| a.line.cmp(&b.line))
    });

    call_flows.sort_by(|a, b| {
        a.caller
            .cmp(&b.caller)
            .then_with(|| a.callee.cmp(&b.callee))
            .then_with(|| a.line.cmp(&b.line))
    });

    Ok(RelationsAnalysis {
        call_flows,
        direct_relations,
        entry_points,
    })
}

pub(crate) fn collect_impact(
    db: &GraphDb,
    candidates: &[SymbolLocation],
) -> Result<(Option<String>, Vec<ExploreImpact>), ExploreError> {
    let mut transitive_consumers = Vec::new();
    let impact_summary = if let Some(primary) = candidates.first() {
        if let Some(sym_id) = primary.symbol.id {
            let impact = query::get_impact(db, sym_id, 3)?;
            let total_callers = impact.total_affected;
            let total_files = impact.affected_files.len();

            for item in impact.affected_symbols {
                let is_cross_repo = item.symbol.repo != primary.symbol.repo;
                transitive_consumers.push(ExploreImpact {
                    symbol_name: item.symbol.name,
                    repo: item.symbol.repo,
                    file_path: item.file_path,
                    depth: item.depth,
                    path_via: item.path_via,
                    cross_repo: is_cross_repo,
                });
            }
            transitive_consumers.sort_by(|a, b| {
                a.depth
                    .cmp(&b.depth)
                    .then_with(|| a.repo.cmp(&b.repo))
                    .then_with(|| a.file_path.cmp(&b.file_path))
                    .then_with(|| a.symbol_name.cmp(&b.symbol_name))
            });

            if total_callers == 0 {
                Some(format!(
                    "Modifying '{}' has no known downstream callers.",
                    primary.symbol.name
                ))
            } else {
                let caller_str = if total_callers == 1 {
                    "1 caller"
                } else {
                    &format!("{total_callers} callers")
                };
                let file_str = if total_files == 1 {
                    "1 file"
                } else {
                    &format!("{total_files} files")
                };
                Some(format!(
                    "Modifying '{}' directly impacts {} across {}.",
                    primary.symbol.name, caller_str, file_str
                ))
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok((impact_summary, transitive_consumers))
}
