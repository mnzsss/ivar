//! A single-repo prototype split into three parallel subfeatures, integrated
//! into the parent and delivered once.

use super::support::{commit_slice, one_repo_hall, run_ok};
use crate::common::hall_root;

#[test]
fn the_single_repo_prototype_persona_reaches_one_parent_delivery() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "prototype"]);
    run_ok(&root, &["feature", "promote", "prototype", "app"]);
    let children = ["onboarding", "checkout", "profile"];
    for child in children {
        run_ok(
            &root,
            &["feature", "create", child, "--parent", "prototype"],
        );
        run_ok(&root, &["feature", "promote", child, "app"]);
        commit_slice(&root, "app", child, &format!("{child}.dart"));
        run_ok(&root, &["plan", "create", child, "plan"]);
        run_ok(&root, &["plan", "approve", child, "plan"]);
    }

    let status = run_ok(&root, &["feature", "status", "prototype", "--recursive"]);
    let open = status["tree"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["parent"] == "prototype")
        .count();
    assert_eq!(open, 3, "{status}");

    for child in children {
        let integrated = run_ok(&root, &["feature", "integrate", child]);
        assert_eq!(integrated["state"], "integrated", "{integrated}");
    }
    let parent_worktree = root.join(".ivar/repos/app/prototype");
    for child in children {
        assert!(
            parent_worktree.join(format!("{child}.dart")).is_file(),
            "{child} was not integrated into the parent"
        );
    }

    let preview = run_ok(&root, &["feature", "deliver", "prototype", "--preview"]);
    assert_eq!(
        preview["preview"]["repos"].as_array().map(Vec::len),
        Some(1),
        "{preview}"
    );
    assert!(
        preview["preview"].get("tree_blockers").is_none(),
        "{preview}"
    );
}
