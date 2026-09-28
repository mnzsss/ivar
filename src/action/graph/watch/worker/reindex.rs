use crate::action::Failure;
use crate::action::graph::cross_repo;
use crate::action::graph::index;
use crate::action::graph::layer::ensure_layer_indexed;
use crate::action::graph::watch::scopes::Scope;
use crate::action::graph::watch::worker::targets::base_commit_for;
use crate::action::graph::watch::worker::{Target, TargetKind};
use crate::action::progress::Silent;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

pub(super) fn reindex(layout: &Layout, db: &GraphDb, target: &Target) -> Result<(), Failure> {
    match &target.kind {
        TargetKind::Base => {
            let Scope::Base { ref repo } = target.scope else {
                return Err(Failure::failed(
                    "graph.watch_reindex",
                    "Expected base scope for base target",
                ));
            };
            let _lock = crate::action::graph::lock_index(layout)?;
            let outcome =
                index::index_repo(db, repo, target.worktree.as_std_path(), false, &Silent)
                    .map_err(|err| Failure::failed("graph.watch_reindex", err.to_string()))?;

            if outcome.files_indexed > 0 || outcome.files_deleted > 0 {
                cross_repo::link_cross_repo_edges(db)
                    .map_err(|err| Failure::failed("graph.watch_reindex", err.to_string()))?;
            }
            Ok(())
        }
        TargetKind::Layer { promotion_base } => {
            let Scope::Layer { feature, repo } = &target.scope else {
                return Err(Failure::failed(
                    "graph.watch_reindex",
                    "Expected layer scope for layer target",
                ));
            };
            let base = base_commit_for(db, repo, promotion_base.as_deref()).ok_or_else(|| {
                Failure::failed(
                    "graph.watch_reindex",
                    format!("base graph index missing for repo `{repo}`"),
                )
            })?;
            ensure_layer_indexed(db, layout, feature, repo, &target.worktree, &base)?;
            Ok(())
        }
    }
}
