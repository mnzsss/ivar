//! Implementation of the graph clean action.

use crate::action::Ctx;
use crate::action::graph::index::HALL_REPO;
use crate::action::graph::input::CleanInput;
use crate::action::graph::outcome::CleanOutcome;
use crate::action::graph::{lock_index, open_graph_db};
use crate::action::{discover_hall, read_manifest};
use crate::error::{Failure, Outcome, Report};
use crate::store::graph::db::{GraphDb, GraphDbError};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, normalise_graph_path};

/// Executes the graph clean action.
pub fn clean_cmd(ctx: &Ctx, args: CleanInput) -> Outcome<CleanOutcome> {
    let target_count = usize::from(args.all)
        + usize::from(args.repo.is_some())
        + usize::from(args.feature.is_some())
        + usize::from(args.path.is_some());
    if target_count == 0 {
        return Err(Failure::blocked(
            "graph.clean_target_required",
            "Must specify one of `--repo <NAME>`, `--feature <NAME>`, `--path <DIR>` or `--all`.",
        ));
    }
    if target_count > 1 {
        return Err(Failure::blocked(
            "graph.clean_conflict",
            "Cannot specify more than one of `--repo`, `--feature`, `--path` and `--all`.",
        ));
    }

    let layout = discover_hall(ctx)?;
    let _lock = lock_index(&layout)?;
    let db = open_graph_db(ctx)?;

    if args.all {
        let stats = db.clean_all().map_err(|err| clean_failed(&err))?;

        Ok(Report::new(CleanOutcome {
            repo: None,
            feature: None,
            path: None,
            all: true,
            repos_removed: stats.repos_removed,
            files_removed: stats.files_removed,
            symbols_removed: stats.symbols_removed,
            edges_removed: stats.edges_removed,
            usage_removed: stats.usage_removed,
            message: format!(
                "Successfully cleaned entire graph database ({} repos, {} files, {} symbols, {} edges, {} usage events removed).",
                stats.repos_removed,
                stats.files_removed,
                stats.symbols_removed,
                stats.edges_removed,
                stats.usage_removed
            ),
        }))
    } else if let Some(feature) = &args.feature {
        let count = db
            .drop_feature_layers(feature)
            .map_err(|err| clean_failed(&err))?;
        Ok(Report::new(CleanOutcome {
            repo: None,
            feature: Some(feature.clone()),
            path: None,
            all: false,
            repos_removed: count,
            files_removed: 0,
            symbols_removed: 0,
            edges_removed: 0,
            usage_removed: 0,
            message: format!(
                "Successfully removed feature layers for '{feature}' ({count} layers removed)."
            ),
        }))
    } else if let Some(raw) = &args.path {
        clean_hall_path(&layout, &db, raw)
    } else if let Some(repo) = args.repo {
        match db.delete_repo(&repo).map_err(|err| clean_failed(&err))? {
            Some(stats) => Ok(Report::new(CleanOutcome {
                repo: Some(repo.clone()),
                feature: None,
                path: None,
                all: false,
                repos_removed: 1,
                files_removed: stats.files_removed,
                symbols_removed: stats.symbols_removed,
                edges_removed: stats.edges_removed,
                usage_removed: 0,
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

fn clean_hall_path(layout: &Layout, db: &GraphDb, raw: &str) -> Outcome<CleanOutcome> {
    let manifest = read_manifest(layout)?;
    let dir = normalise_graph_path(raw)
        .map_err(|err| Failure::blocked("graph.path_invalid", err.to_string()))?;
    if !manifest.graph_paths().contains(&dir) {
        return Err(Failure::blocked(
            "graph.path_not_configured",
            format!("`{dir}` is not in `graph.paths` of `ivar.json`"),
        ));
    }
    let files_removed = db
        .delete_files_under(HALL_REPO, &dir)
        .map_err(|err| clean_failed(&err))?;
    db.watch_forget(&format!("hall:{dir}"))
        .map_err(|err| clean_failed(&err))?;
    let manifest = manifest.with_graph_path_removed(&dir)?;
    Manifest::write(layout, &manifest)
        .map_err(|err| Failure::failed("manifest.write_failed", err.to_string()))?;
    let repos_removed = if manifest.graph_paths().is_empty() {
        db.delete_repo(HALL_REPO)
            .map_err(|err| clean_failed(&err))?
            .map_or(0, |_| 1)
    } else {
        0
    };
    Ok(Report::new(CleanOutcome {
        repo: None,
        feature: None,
        path: Some(dir.clone()),
        all: false,
        repos_removed,
        files_removed,
        symbols_removed: 0,
        edges_removed: 0,
        usage_removed: 0,
        message: format!(
            "Removed hall path '{dir}' from the graph ({files_removed} files removed)."
        ),
    }))
}

fn clean_failed(err: &GraphDbError) -> Failure {
    Failure::failed("graph.clean_failed", err.to_string())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/clean.rs"]
mod tests;
