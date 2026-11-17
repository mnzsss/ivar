//! A branch whose PR already merged is not delivered as a second PR.

use super::fixture::*;
use super::*;

const MERGED_URL: &str = "https://github.com/acme/pull/9";

fn bare(root: &Utf8Path, repo: &str) -> Utf8PathBuf {
    Layout::at(root.to_path_buf()).repo_bare(&RepoName::new(repo).unwrap())
}

fn worktree(root: &Utf8Path, repo: &str) -> Utf8PathBuf {
    Layout::at(root.to_path_buf()).repo_worktree(
        &RepoName::new(repo).unwrap(),
        &BranchName::new("checkout").unwrap(),
    )
}

/// The branch tip of `repo`, as the PR that merged it would have recorded.
fn tip(root: &Utf8Path, repo: &str) -> String {
    git_stdout(&worktree(root, repo), &["rev-parse", "HEAD"])
        .trim()
        .to_owned()
}

/// One more commit on `repo`'s branch, after the PR merged.
fn commit_after_merge(root: &Utf8Path, repo: &str) {
    let worktree = worktree(root, repo);
    std::fs::write(worktree.join("after.md"), "after\n").unwrap();
    git(&worktree, &["add", "after.md"]);
    git(&worktree, &["commit", "-m", "after the merge"]);
}

/// Seed a PR for `api`'s branch in `state`, pinned to `head`.
fn seed(fake: &FakeGh, root: &Utf8Path, state: &str, head: &str) {
    fake.set_existing_pr(&bare(root, "api"), "checkout", MERGED_URL, "main", state);
    fake.set_pr_head(MERGED_URL, head);
}

fn blockers(preview: &serde_json::Value, index: usize) -> Vec<String> {
    preview["preview"]["repos"][index]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|blocker| blocker.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_merged_pr_whose_head_the_branch_carries_blocks_the_preview() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    seed(&fake, &root, "MERGED", &tip(&root, "api"));
    commit_after_merge(&root, "api");

    let preview = preview_json(&root, preview_input("checkout"));

    let listed = blockers(&preview, 0);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0].contains("deliver.pr_already_merged"),
        "{listed:?}"
    );
    assert!(listed[0].contains("PR #9"), "{listed:?}");
    let merge_commit = git_stdout(&origin_dir(&root, "api"), &["rev-parse", "main"]);
    assert!(listed[0].contains(merge_commit.trim()), "{listed:?}");
    assert!(listed[0].contains("--only"), "{listed:?}");
    assert!(listed[0].contains("new feature"), "{listed:?}");
    assert_eq!(preview["preview"]["repos"][0]["merged_pr"]["number"], 9);
    // The same listing is what the human preview prints and apply refuses on.
    assert_eq!(preview["blockers"].as_array().unwrap().len(), 1);
}

/// Where the hall's fixture created `repo`'s origin, which `as_github_remotes`
/// keeps reachable behind its GitHub URL.
fn origin_dir(root: &Utf8Path, repo: &str) -> Utf8PathBuf {
    root.parent().unwrap().join("origins").join(repo)
}

#[test]
fn apply_refuses_with_its_own_code_and_neither_pushes_nor_opens_a_pr() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    seed(&fake, &root, "MERGED", &tip(&root, "api"));
    commit_after_merge(&root, "api");
    let fingerprint = preview_json(&root, preview_input("checkout"))["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    let failure = deliver(
        &Ctx::new(root.clone()),
        apply_input("checkout", &fingerprint),
    )
    .unwrap_err();

    assert_eq!(failure.code, "deliver.pr_already_merged");
    assert!(
        failure
            .actual
            .as_deref()
            .unwrap_or_default()
            .contains("PR #9"),
        "{failure:?}"
    );
    let fixes: Vec<&str> = failure.fix_actions.iter().map(|fix| fix.code).collect();
    assert!(
        fixes.contains(&"deliver.new_feature_from_default"),
        "{fixes:?}"
    );
    assert!(fixes.contains(&"deliver.only_without_repo"), "{fixes:?}");
    assert!(!fake.log().contains("pr create"), "{}", fake.log());
    assert!(
        remote_ref(origin_dir(&root, "api").as_str(), "checkout").is_none(),
        "nothing may be pushed"
    );
}

#[test]
fn an_open_pr_still_updates_and_no_pr_still_opens_a_new_one() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let none = preview_json(&root, preview_input("checkout"));
    assert_eq!(none["preview"]["repos"][0]["action"], "new_pr");
    assert!(blockers(&none, 0).is_empty());
    assert!(none["preview"]["repos"][0].get("merged_pr").is_none());

    seed(&fake, &root, "OPEN", &tip(&root, "api"));
    let open = preview_json(&root, preview_input("checkout"));
    assert_eq!(open["preview"]["repos"][0]["action"], "update_pr");
    assert!(blockers(&open, 0).is_empty());
    assert!(fake.log().contains("--state all"), "{}", fake.log());
}

#[test]
fn a_closed_pr_that_never_merged_does_not_block() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    seed(&fake, &root, "CLOSED", &tip(&root, "api"));
    commit_after_merge(&root, "api");

    let preview = preview_json(&root, preview_input("checkout"));

    assert!(blockers(&preview, 0).is_empty());
    assert_eq!(preview["preview"]["repos"][0]["action"], "new_pr");
}

#[test]
fn a_branch_recreated_after_the_merge_is_not_blocked() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    // The merged PR's head is a commit this branch never descended from.
    let unrelated = git_stdout(
        &bare(&root, "api"),
        &[
            "commit-tree",
            "main^{tree}",
            "-m",
            "old head of a branch with the same name",
        ],
    );
    seed(&fake, &root, "MERGED", unrelated.trim());

    let preview = preview_json(&root, preview_input("checkout"));

    assert!(blockers(&preview, 0).is_empty(), "{preview}");
    assert_eq!(preview["preview"]["repos"][0]["action"], "new_pr");
}

#[test]
fn a_head_missing_from_the_clone_cannot_prove_the_branch_is_new() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    seed(&fake, &root, "MERGED", &"1".repeat(40));

    let preview = preview_json(&root, preview_input("checkout"));

    let listed = blockers(&preview, 0);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0].contains("deliver.pr_already_merged"),
        "{listed:?}"
    );
}

#[test]
fn only_without_the_merged_repo_lets_the_others_deliver() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);
    seed(&fake, &root, "MERGED", &tip(&root, "api"));
    commit_after_merge(&root, "api");
    let only_web = DeliverInput {
        only: vec!["web".to_owned()],
        ..preview_input("checkout")
    };

    let applied = deliver_json(&root, only_web);

    assert_eq!(applied["pushes"].as_array().unwrap().len(), 1);
    assert_eq!(applied["pushes"][0]["repo"], "web");
    assert!(remote_ref(origin_dir(&root, "api").as_str(), "checkout").is_none());
}

#[test]
fn a_push_only_remote_and_land_mode_never_call_gh() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);

    // The origin is a local path: push-only.
    let preview = preview_json(&root, preview_input("checkout"));
    assert_eq!(preview["preview"]["repos"][0]["action"], "push_only");

    as_github_remotes(&root);
    let land = preview_json(&root, land_preview_input("checkout"));
    assert_eq!(land["preview"]["repos"][0]["action"], "land_on_default");

    assert_eq!(fake.log(), "", "gh must not have been called");
}
