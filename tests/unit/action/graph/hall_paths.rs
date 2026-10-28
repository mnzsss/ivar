#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Providers, Repo};

fn hall_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar/repos/api/main")).unwrap();
    for (path, content) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    (tmp, root)
}

fn manifest(repos: &[&str]) -> Manifest {
    Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos
            .iter()
            .map(|r| {
                Repo::new(
                    RepoName::new(*r).unwrap(),
                    format!("git@x:{r}.git"),
                    BranchName::new("main").unwrap(),
                )
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn validate_hall_path_accepts_a_hall_dir_and_names_each_refusal() {
    let (_tmp, root) = hall_with(&[("brand/a.md", "a"), ("docs/b.md", "b")]);
    std::os::unix::fs::symlink(root.join(".ivar/repos/api/main"), root.join("escape")).unwrap();
    let layout = Layout::at(root.clone());
    let m = manifest(&["api"]);

    assert_eq!(validate_hall_path(&layout, &m, "brand/").unwrap(), "brand");
    assert_eq!(
        validate_hall_path(&layout, &m, "../x").unwrap_err().code,
        "graph.path_invalid"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, ".").unwrap_err().code,
        "graph.path_invalid"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, ".ivar/repos")
            .unwrap_err()
            .code,
        "graph.path_invalid"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, "nowhere").unwrap_err().code,
        "graph.path_missing"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, "escape").unwrap_err().code,
        "graph.path_in_repo"
    );
    assert_eq!(
        validate_hall_path(&layout, &manifest(&["hall"]), "brand")
            .unwrap_err()
            .code,
        "graph.path_repo_clash"
    );
}

#[test]
fn validate_hall_path_refuses_a_symlink_that_leaves_the_hall() {
    let (_tmp, root) = hall_with(&[("brand/a.md", "a")]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("away")).unwrap();
    let layout = Layout::at(root);

    assert_eq!(
        validate_hall_path(&layout, &manifest(&["api"]), "away")
            .unwrap_err()
            .code,
        "graph.path_outside_hall"
    );
}

#[test]
fn validate_hall_path_accepts_the_hall_skills_and_nothing_else_in_ivar() {
    let (_tmp, root) = hall_with(&[
        (".ivar/skills/metrics/SKILL.md", "# skill"),
        (".ivar/state.json", "{}"),
    ]);
    let layout = Layout::at(root.clone());
    let m = manifest(&["api"]);

    assert_eq!(
        validate_hall_path(&layout, &m, ".ivar/skills").unwrap(),
        ".ivar/skills"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, ".ivar/skills/metrics").unwrap(),
        ".ivar/skills/metrics"
    );
    assert_eq!(
        validate_hall_path(&layout, &m, ".ivar").unwrap_err().code,
        "graph.path_invalid"
    );
}
