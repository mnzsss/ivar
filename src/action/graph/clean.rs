//! Implementation of the graph clean action.

use crate::action::Ctx;
use crate::action::graph::input::CleanInput;
use crate::action::graph::open_graph_db;
use crate::action::graph::outcome::CleanOutcome;
use crate::error::{Failure, Outcome, Report};

/// Executes the graph clean action.
pub fn clean_cmd(ctx: &Ctx, args: CleanInput) -> Outcome<CleanOutcome> {
    if !args.all && args.repo.is_none() {
        return Err(Failure::blocked(
            "graph.clean_target_required",
            "Must specify either `--repo <NAME>` to clean a repository or `--all` to clean the entire graph.",
        ));
    }
    if args.all && args.repo.is_some() {
        return Err(Failure::blocked(
            "graph.clean_conflict",
            "Cannot specify both `--repo` and `--all`.",
        ));
    }

    let db = open_graph_db(ctx)?;

    if args.all {
        let stats = db
            .clean_all()
            .map_err(|err| Failure::failed("graph.clean_failed", err.to_string()))?;

        Ok(Report::new(CleanOutcome {
            repo: None,
            all: true,
            repos_removed: stats.repos_removed,
            files_removed: stats.files_removed,
            symbols_removed: stats.symbols_removed,
            edges_removed: stats.edges_removed,
            message: format!(
                "Successfully cleaned entire graph database ({} repos, {} files, {} symbols, {} edges removed).",
                stats.repos_removed, stats.files_removed, stats.symbols_removed, stats.edges_removed
            ),
        }))
    } else if let Some(repo) = args.repo {
        match db
            .delete_repo(&repo)
            .map_err(|err| Failure::failed("graph.clean_failed", err.to_string()))?
        {
            Some(stats) => Ok(Report::new(CleanOutcome {
                repo: Some(repo.clone()),
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
