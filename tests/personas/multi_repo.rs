//! A feature in a three-repo hall that touches two of them.

use super::support::{commit_slice, hall_with_repos, run_ok};
use crate::common::hall_root;

#[test]
fn the_multi_repo_persona_delivers_only_the_repos_it_touched() {
    let (_guard, root) = hall_root();
    hall_with_repos(&root, &["api", "web", "docs"]);
    run_ok(&root, &["feature", "create", "checkout"]);
    for repo in ["api", "web"] {
        run_ok(&root, &["feature", "promote", "checkout", repo]);
        commit_slice(&root, repo, "checkout", "checkout.txt");
    }
    run_ok(&root, &["plan", "create", "checkout", "plan"]);
    run_ok(&root, &["plan", "approve", "checkout", "plan"]);

    let preview = run_ok(&root, &["feature", "deliver", "checkout", "--preview"]);
    let mut repos: Vec<&str> = preview["preview"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["repo"].as_str().unwrap())
        .collect();
    repos.sort_unstable();
    assert_eq!(repos, ["api", "web"], "{preview}");
}
