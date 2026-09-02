//! Inline and file-backed pull-request body behavior.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, ivar_on_github, preview_on_github_with,
    setup_deliver_hall,
};
use predicates::prelude::*;

/// Inline and cwd-relative md/txt body: `./body.md` and `body.txt` are resolved
/// correctly.
#[test]
fn inline_and_file_bodies() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Write body file
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "Content from file\n").unwrap();
    let body_txt = root.join("body.txt");
    std::fs::write(&body_txt, "Content from txt\n").unwrap();

    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Inline body (just text, no ./ prefix or .md/.txt extension)
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "inline body text",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    let repo = &preview["repos"][0];
    assert_eq!(repo["pr_title"], "feat", "title should be set");
    assert_eq!(
        repo["pr_body"], "inline body text",
        "inline body should be stored"
    );

    // File body with ./ prefix for .md
    let output2 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "./body.md",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value2: serde_json::Value = serde_json::from_slice(&output2).expect("valid json");
    let preview2 = &value2["preview"];
    let repo2 = &preview2["repos"][0];
    assert_eq!(
        repo2["pr_body"], "Content from file\n",
        "file body ./body.md should resolve to file content"
    );

    // File body with ./ prefix for .txt
    let output3 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "./body.txt",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value3: serde_json::Value = serde_json::from_slice(&output3).expect("valid json");
    let preview3 = &value3["preview"];
    let repo3 = &preview3["repos"][0];
    assert_eq!(
        repo3["pr_body"], "Content from txt\n",
        "file body ./body.txt should resolve to file content"
    );

    // Non-prefixed body.txt is treated as inline text, not a file
    let output4 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "body.md",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value4: serde_json::Value = serde_json::from_slice(&output4).expect("valid json");
    let preview4 = &value4["preview"];
    let repo4 = &preview4["repos"][0];
    assert_eq!(
        repo4["pr_body"], "body.md",
        "non-prefixed body.md should be treated as inline text"
    );

    // An absolute path to a .md/.txt file resolves to its content. Without
    // this, the path itself silently becomes the pull request body -- the
    // argument is path-shaped, so inline text is never what was meant.
    let output5 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            body_md.as_str(),
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value5: serde_json::Value = serde_json::from_slice(&output5).expect("valid json");
    let repo5 = &value5["preview"]["repos"][0];
    assert_eq!(
        repo5["pr_body"], "Content from file\n",
        "an absolute .md path should resolve to file content"
    );
}
/// Body file change invalidates fingerprint: changing the content of a body
/// file and re-applying should be rejected by the fingerprint gate.
#[test]
fn body_file_change_invalidates_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Write initial body file
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "original content\n").unwrap();

    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Preview with body file to get fingerprint
    let preview = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat", "--body", "./body.md"],
    );
    let fp = preview["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Modify the body file content AFTER the preview
    std::fs::write(&body_md, "modified content\n").unwrap();

    // Apply with old fingerprint should fail (drift detection)
    ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--name",
            "feat",
            "--body",
            "./body.md",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}
