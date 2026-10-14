//! Drops graph state the hall no longer has: repos outside the manifest,
//! layers of features without a directory, watch scopes without a target.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::infra::fs;
use crate::store::graph::db::{GraphDb, GraphDbError};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::index::HALL_REPO;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RemovedLayer {
    pub feature: String,
    pub repo: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Removed {
    pub repos: Vec<String>,
    pub layers: Vec<RemovedLayer>,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub hall_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Removed {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
            && self.layers.is_empty()
            && self.scopes.is_empty()
            && self.hall_files == 0
    }
}

/// What the hall declares and has on disk, read once per reconciliation.
#[derive(Debug)]
pub struct HallFacts {
    declared: BTreeSet<String>,
    paths: BTreeSet<String>,
    features: Result<BTreeSet<String>, String>,
}

impl HallFacts {
    #[must_use]
    pub fn gather(layout: &Layout, manifest: &Manifest) -> Self {
        let features_dir = layout.features_dir();
        let features = if features_dir.as_std_path().exists() {
            fs::read_dir(&features_dir)
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter_map(|p| p.file_name().map(str::to_owned))
                        .collect()
                })
                .map_err(|e| format!("could not read `{features_dir}`: {e}"))
        } else {
            Ok(BTreeSet::new())
        };
        Self {
            declared: manifest
                .repos()
                .iter()
                .map(|r| r.name().to_string())
                .collect(),
            paths: manifest.graph_paths().iter().cloned().collect(),
            features,
        }
    }

    #[must_use]
    pub fn features_unreadable(&self) -> Option<&str> {
        self.features.as_ref().err().map(String::as_str)
    }

    #[must_use]
    pub fn layer_is_live(&self, feature: &str) -> bool {
        match &self.features {
            Ok(features) => features.contains(feature),
            Err(_) => true,
        }
    }

    #[must_use]
    pub fn repo_is_declared(&self, repo: &str) -> bool {
        self.declared.contains(repo)
    }

    #[must_use]
    pub fn scope_is_live(&self, key: &str) -> bool {
        if let Some(repo) = key.strip_prefix("base:") {
            return self.repo_is_declared(repo);
        }
        if let Some(dir) = key.strip_prefix("hall:") {
            return self.paths.contains(dir);
        }
        if let Some(rest) = key.strip_prefix("layer:") {
            return rest
                .split_once(':')
                .is_some_and(|(feature, _)| self.layer_is_live(feature));
        }
        false
    }
}

/// # Errors
///
/// Returns [`GraphDbError`] when a listing or a drop fails; every drop is its
/// own transaction, so a partial run leaves the graph consistent.
pub fn reconcile(
    db: &GraphDb,
    layout: &Layout,
    manifest: &Manifest,
) -> Result<Removed, GraphDbError> {
    let facts = HallFacts::gather(layout, manifest);
    let mut removed = Removed::default();
    for row in db.list_base_repos()? {
        if row.id != HALL_REPO && !facts.repo_is_declared(&row.id) {
            db.forget_repo(&row.id)?;
            removed.repos.push(row.id);
        }
    }
    if db.get_repo(HALL_REPO)?.is_some() {
        if facts.paths.is_empty() {
            db.forget_repo(HALL_REPO)?;
            removed.repos.push(HALL_REPO.to_owned());
        } else {
            for file in db.get_files_for_repo(HALL_REPO)? {
                let kept = facts
                    .paths
                    .iter()
                    .any(|dir| file.path.starts_with(&format!("{dir}/")));
                if !kept {
                    db.delete_file(HALL_REPO, &file.path)?;
                    removed.hall_files += 1;
                }
            }
        }
    }
    match facts.features_unreadable() {
        Some(reason) => removed.skipped = Some(format!("layers kept: {reason}")),
        None => {
            let mut dropped: BTreeSet<String> = BTreeSet::new();
            for layer in db.list_layer_records()? {
                if facts.layer_is_live(&layer.feature) {
                    continue;
                }
                if dropped.insert(layer.feature.clone()) {
                    db.drop_feature_layers(&layer.feature)?;
                }
                removed.layers.push(RemovedLayer {
                    feature: layer.feature,
                    repo: layer.repo,
                });
            }
        }
    }
    for row in db.watch_scopes()? {
        if !facts.scope_is_live(&row.scope) {
            db.watch_forget(&row.scope)?;
            removed.scopes.push(row.scope);
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/reconcile.rs"]
mod tests;
