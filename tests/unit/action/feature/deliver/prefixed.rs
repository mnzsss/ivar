//! A repo stored inside the hall's own remote under `repos/<name>/`.
//! Everything deliver shows a human, and everything it writes, lives under
//! that prefix and never in the hall remote's root branch namespace.

use super::fixture::*;
use super::*;

fn human(report: &Report<DeliverOutcome>) -> String {
    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string()
}

fn notes_worktree(root: &Utf8PathBuf, branch: &str) -> Utf8PathBuf {
    Layout::at(root.clone()).repo_worktree(
        &RepoName::new("notes").unwrap(),
        &BranchName::new(branch).unwrap(),
    )
}

#[test]
fn a_prefixed_repo_previews_the_refs_it_pushes() {
    let (_guard, root) = hall_with_prefixed_promoted("notes");
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert_eq!(
        repo.push_refspec,
        "checkout:refs/heads/repos/notes/checkout"
    );
    assert_eq!(repo.ref_prefix.as_deref(), Some("repos/notes/"));
    assert_eq!(
        repo.base_branch.as_str(),
        "main",
        "base_branch stays the local name"
    );
    let rendered = human(&report);
    assert!(
        rendered.contains("refspec: checkout:refs/heads/repos/notes/checkout"),
        "{rendered}"
    );
    assert!(rendered.contains("base:    repos/notes/main"), "{rendered}");
}

#[test]
fn a_prefixed_repo_land_preview_pushes_the_prefixed_default() {
    let (_guard, root) = hall_with_prefixed_promoted("notes");
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, land_preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert_eq!(
        repo.default_branch.as_ref().map(BranchName::as_str),
        Some("main"),
        "default_branch stays the local name"
    );
    let rendered = human(&report);
    assert!(
        rendered.contains("checkout -> main"),
        "the local merge keeps local names: {rendered}"
    );
    assert!(
        rendered.contains("then push repos/notes/main to"),
        "{rendered}"
    );
}

#[test]
fn applying_a_prefixed_delivery_pushes_under_the_prefix_only() {
    let (_guard, root) = hall_with_prefixed_promoted("notes");
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let origin = origin_of(&root, "notes");
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();

    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    assert!(
        report.value.pushes[0].ok,
        "{:?}",
        report.value.pushes[0].detail
    );
    assert!(remote_ref(&origin, "repos/notes/checkout").is_some());
    assert_eq!(
        remote_ref(&origin, "checkout"),
        None,
        "nothing lands in the hall remote's root namespace"
    );
}

#[test]
fn landing_a_prefixed_repo_moves_its_prefixed_default_and_not_the_halls() {
    let (_guard, root) = hall_with_prefixed_promoted("notes");
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let origin = origin_of(&root, "notes");
    let hall_main_before = remote_ref(&origin, "main");
    let feature_tip = git_stdout(&notes_worktree(&root, "checkout"), &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    let preview = deliver(&ctx, land_preview_input("checkout")).unwrap();

    let out = deliver(
        &ctx,
        DeliverInput {
            preview: false,
            fingerprint: Some(preview.value.preview.fingerprint.clone()),
            ..land_preview_input("checkout")
        },
    )
    .unwrap();

    let landed = &out.value.land[0];
    assert!(landed.merged && landed.pushed, "{:?}", landed.detail);
    let prefixed_main = remote_ref(&origin, "repos/notes/main").expect("the prefixed default");
    assert!(prefixed_main.starts_with(&feature_tip), "{prefixed_main}");
    assert_eq!(
        remote_ref(&origin, "main"),
        hall_main_before,
        "the hall's own main is untouched"
    );
}

#[test]
fn a_rejected_prefixed_push_names_the_prefixed_ref_in_its_recovery() {
    let (_guard, root) = hall_with_prefixed_promoted("notes");
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();
    git(
        &notes_worktree(&root, "checkout"),
        &["commit", "--amend", "-m", "work, rewritten"],
    );

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    let push = &report.value.pushes[0];
    assert!(!push.ok);
    let command = push
        .fix
        .as_ref()
        .and_then(|fix| fix.command.as_deref())
        .expect("a recovery command");
    assert!(
        command.contains("--force-with-lease=refs/heads/repos/notes/checkout:"),
        "the lease names the prefixed ref and its tip: {command}"
    );
    assert!(
        command.ends_with(" checkout:refs/heads/repos/notes/checkout"),
        "{command}"
    );
}
