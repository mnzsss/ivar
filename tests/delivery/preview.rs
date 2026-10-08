use crate::common::{hall_root, ivar};
use crate::support::setup_deliver_hall;

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(
        value["ok"], true,
        "the --json envelope carries the success flag"
    );
    let preview = &value["preview"];
    assert_eq!(preview["feature"], "checkout");
    let repos = preview["repos"].as_array().expect("repos is an array");
    assert_eq!(repos.len(), 1);
    let repo = &repos[0];
    assert_eq!(repo["repo"], "api");
    assert_eq!(repo["local_branch"], "checkout");
    assert!(repo["remote"].as_str().unwrap().contains("origins/api"));
    assert_eq!(repo["push_refspec"], "checkout:refs/heads/checkout");
    // The remote is a local path, not GitHub — push only, no PR surface.
    assert_eq!(repo["action"], "push_only");
    assert_eq!(repo["base_branch"], "main");
    assert!(repo["dependencies"].is_array());
    // Unpushed commits are the work deliver carries, so pending, not a blocker.
    assert_eq!(repo["blockers"], serde_json::json!([]));
    let pending = repo["pending"].as_array().expect("pending is an array");
    assert!(
        pending
            .iter()
            .any(|p| p.as_str().unwrap_or("").contains("not pushed")),
        "expected the unpushed commits as pending, got: {pending:?}"
    );
    // No pr_url in preview (nothing created yet).
    assert!(repo["pr_url"].is_null());
    // Fingerprint is present and non-empty.
    assert!(
        preview["fingerprint"].as_str().unwrap().len() == 64,
        "fingerprint must be a sha-256 hex digest"
    );
}
