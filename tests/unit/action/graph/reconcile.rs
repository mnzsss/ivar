#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};

fn hall() -> (tempfile::TempDir, Layout, Manifest) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);
    fs::ensure_dir(&layout.features_dir()).unwrap();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            "git@x:api.git",
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    (tmp, layout, manifest)
}

fn feature_on_disk(layout: &Layout, name: &str, promoted: &str) {
    let mut feature = Feature::new(
        FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    feature.promote(RepoName::new(promoted).unwrap());
    feature.write(layout).unwrap();
}

#[test]
fn a_repo_not_in_the_manifest_is_forgotten_with_its_layers_and_scope() {
    let (_tmp, layout, manifest) = hall();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.insert_repo("orca", "/orca", "main", None).unwrap();
    let id = db
        .ensure_layer_record("feat", "orca", "/f/orca", "c1")
        .unwrap();
    db.insert_repo(&format!("orca/{id}"), "/f/orca", "feat", None)
        .unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_register("base:orca").unwrap();

    let removed = reconcile(&db, &layout, &manifest).unwrap();

    assert_eq!(removed.repos, vec!["orca"]);
    assert_eq!(removed.scopes, vec!["base:orca"]);
    assert!(db.get_repo("orca").unwrap().is_none());
    assert!(db.get_repo("api").unwrap().is_some());
    let rows: Vec<String> = db
        .watch_scopes()
        .unwrap()
        .into_iter()
        .map(|r| r.scope)
        .collect();
    assert_eq!(rows, vec!["base:api"]);
}

#[test]
fn a_layer_whose_feature_directory_is_gone_is_dropped_and_a_live_feature_keeps_layer_and_scope_without_a_worktree()
 {
    let (_tmp, layout, manifest) = hall();
    feature_on_disk(&layout, "alive", "api");
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    let dead = db
        .ensure_layer_record("dead", "api", "/d/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{dead}"), "/d/api", "dead", None)
        .unwrap();
    let alive = db
        .ensure_layer_record("alive", "api", "/a/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{alive}"), "/a/api", "alive", None)
        .unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_register("layer:dead:api").unwrap();
    db.watch_register("layer:alive:api").unwrap();

    let removed = reconcile(&db, &layout, &manifest).unwrap();

    assert_eq!(
        removed.layers,
        vec![RemovedLayer {
            feature: "dead".into(),
            repo: "api".into()
        }]
    );
    assert_eq!(removed.scopes, vec!["layer:dead:api"]);
    assert!(db.get_layer_record("alive", "api").unwrap().is_some());
    let rows: Vec<String> = db
        .watch_scopes()
        .unwrap()
        .into_iter()
        .map(|r| r.scope)
        .collect();
    assert_eq!(rows, vec!["base:api", "layer:alive:api"]);
}

#[test]
fn the_hall_repo_is_never_treated_as_missing_from_the_manifest() {
    let (_tmp, layout, manifest) = hall();
    let with_brand = manifest.with_graph_path_added("brand").unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("hall", "/hall", "", None).unwrap();
    db.upsert_file("hall", "brand/DESIGN.md", "h1", 1, 1)
        .unwrap();

    let removed = reconcile(&db, &layout, &with_brand).unwrap();

    assert!(removed.is_empty());
    assert!(db.get_repo("hall").unwrap().is_some());
}

#[test]
fn an_unreadable_features_directory_keeps_every_layer_and_says_why() {
    let (_tmp, layout, manifest) = hall();
    std::fs::remove_dir_all(layout.features_dir()).unwrap();
    std::fs::write(layout.features_dir(), b"not a directory").unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    let id = db
        .ensure_layer_record("maybe", "api", "/m/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{id}"), "/m/api", "maybe", None)
        .unwrap();
    db.watch_register("layer:maybe:api").unwrap();
    db.watch_register("base:gone").unwrap();

    let removed = reconcile(&db, &layout, &manifest).unwrap();

    assert!(removed.layers.is_empty());
    assert_eq!(
        removed.scopes,
        vec!["base:gone"],
        "base scopes are still reconciled"
    );
    assert!(
        removed
            .skipped
            .as_deref()
            .is_some_and(|s| s.contains("features")),
        "{:?}",
        removed.skipped
    );
    assert!(db.get_layer_record("maybe", "api").unwrap().is_some());
}

#[test]
fn an_absent_features_directory_means_no_features_not_unknown() {
    let (_tmp, layout, manifest) = hall();
    std::fs::remove_dir_all(layout.features_dir()).unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    let id = db
        .ensure_layer_record("gone", "api", "/g/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{id}"), "/g/api", "gone", None)
        .unwrap();

    let removed = reconcile(&db, &layout, &manifest).unwrap();

    assert_eq!(removed.layers.len(), 1);
    assert!(removed.skipped.is_none());
}

#[test]
fn one_run_drops_an_orphan_repo_layer_and_scope_together() {
    let (_tmp, layout, manifest) = hall();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.insert_repo("orca", "/orca", "main", None).unwrap();
    let id = db
        .ensure_layer_record("dead", "api", "/d/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{id}"), "/d/api", "dead", None)
        .unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_register("base:orca").unwrap();
    db.watch_register("layer:dead:api").unwrap();
    db.watch_register("hall:brand").unwrap();

    let removed = reconcile(&db, &layout, &manifest).unwrap();

    assert_eq!(removed.repos, vec!["orca"]);
    assert_eq!(removed.layers.len(), 1);
    assert_eq!(
        removed.scopes,
        vec!["base:orca", "hall:brand", "layer:dead:api"]
    );
}

#[test]
fn reconciling_twice_changes_nothing_the_second_time() {
    let (_tmp, layout, manifest) = hall();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.insert_repo("orca", "/orca", "main", None).unwrap();
    db.watch_register("base:orca").unwrap();
    db.watch_register("base:api").unwrap();

    assert!(!reconcile(&db, &layout, &manifest).unwrap().is_empty());
    assert!(reconcile(&db, &layout, &manifest).unwrap().is_empty());
}

#[test]
fn hall_facts_answer_liveness_per_scope_kind() {
    let (_tmp, layout, manifest) = hall();
    feature_on_disk(&layout, "alive", "api");
    let facts = HallFacts::gather(&layout, &manifest);

    assert!(facts.scope_is_live("base:api"));
    assert!(!facts.scope_is_live("base:orca"));
    assert!(facts.repo_is_declared("api"));
    assert!(!facts.repo_is_declared("orca"));
    assert!(facts.scope_is_live("layer:alive:api"));
    assert!(!facts.scope_is_live("layer:dead:api"));
    assert!(!facts.scope_is_live("hall:brand"));
    assert!(!facts.scope_is_live("bogus"));
    assert!(facts.layer_is_live("alive") && !facts.layer_is_live("dead"));
}

#[test]
fn hall_files_outside_the_configured_paths_are_dropped_and_the_repo_goes_with_the_last_path() {
    let (_tmp, layout, manifest) = hall();
    let with_docs = manifest.with_graph_path_added("docs").unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("hall", "/hall", "", None).unwrap();
    db.upsert_file("hall", "brand/DESIGN.md", "h1", 1, 1)
        .unwrap();
    db.upsert_file("hall", "docs/a.md", "h2", 1, 1).unwrap();

    let removed = reconcile(&db, &layout, &with_docs).unwrap();
    assert_eq!(removed.hall_files, 1);
    assert!(db.get_file("hall", "brand/DESIGN.md").unwrap().is_none());
    assert!(db.get_file("hall", "docs/a.md").unwrap().is_some());

    let removed = reconcile(&db, &layout, &manifest).unwrap();
    assert_eq!(removed.repos, vec!["hall"]);
    assert!(db.get_repo("hall").unwrap().is_none());
}
