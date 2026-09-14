//! Implementation of the graph clean action.

use crate::action::Ctx;
use crate::action::graph::input::CleanInput;
use crate::action::graph::open_graph_db;
use crate::action::graph::outcome::CleanOutcome;
use crate::error::{Failure, Outcome, Report};

/// Executes the graph clean action.
pub fn clean_cmd(ctx: &Ctx, args: CleanInput) -> Outcome<CleanOutcome> {
    let target_count = usize::from(args.all)
        + usize::from(args.repo.is_some())
        + usize::from(args.feature.is_some());
    if target_count == 0 {
        return Err(Failure::blocked(
            "graph.clean_target_required",
            "Must specify either `--repo <NAME>` to clean a repository, `--feature <NAME>` to clean feature layers, or `--all` to clean the entire graph.",
        ));
    }
    if target_count > 1 {
        return Err(Failure::blocked(
            "graph.clean_conflict",
            "Cannot specify more than one of `--repo`, `--feature`, and `--all`.",
        ));
    }

    let db = open_graph_db(ctx)?;

    if args.all {
        let stats = db
            .clean_all()
            .map_err(|err| Failure::failed("graph.clean_failed", err.to_string()))?;

        Ok(Report::new(CleanOutcome {
            repo: None,
            feature: None,
            all: true,
            repos_removed: stats.repos_removed,
            files_removed: stats.files_removed,
            symbols_removed: stats.symbols_removed,
            edges_removed: stats.edges_removed,
            message: format!(
                "Successfully cleaned entire graph database ({} repos, {} files, {} symbols, {} edges removed).",
                stats.repos_removed,
                stats.files_removed,
                stats.symbols_removed,
                stats.edges_removed
            ),
        }))
    } else if let Some(feature) = &args.feature {
        let count = db
            .drop_feature_layers(feature)
            .map_err(|err| Failure::failed("graph.clean_failed", err.to_string()))?;
        Ok(Report::new(CleanOutcome {
            repo: None,
            feature: Some(feature.clone()),
            all: false,
            repos_removed: count,
            files_removed: 0,
            symbols_removed: 0,
            edges_removed: 0,
            message: format!(
                "Successfully removed feature layers for '{feature}' ({count} layers removed)."
            ),
        }))
    } else if let Some(repo) = args.repo {
        match db
            .delete_repo(&repo)
            .map_err(|err| Failure::failed("graph.clean_failed", err.to_string()))?
        {
            Some(stats) => Ok(Report::new(CleanOutcome {
                repo: Some(repo.clone()),
                feature: None,
                all: false,
                repos_removed: 1,
                files_removed: stats.files_removed,
                symbols_removed: stats.symbols_removed,
                edges_removed: stats.edges_removed,
                message: format!(
                    "Successfully removed repository '{repo}' from graph ({} files, {} symbols, {} edges removed).",
                    stats.files_removed, stats.symbols_removed, stats.edges_removed
                ),
            })),
            None => Err(Failure::blocked(
                "graph.repo_not_found",
                format!("Repository '{repo}' is not present in the graph index."),
            )),
        }
    } else {
        unreachable!()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/clean.rs"]
mod tests;
