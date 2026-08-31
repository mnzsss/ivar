use super::fixture::*;
use super::*;

#[test]
fn deliver_refuses_a_child_with_the_integrate_command() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    child_of_checkout(&root, "child");

    // Preview and apply refuse identically.
    let failure = deliver(&ctx, preview_input("child")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature integrate child")
    );
    let failure = deliver(&ctx, apply_input("child", "whatever")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
}

#[test]
fn github_repo_in_land_mode_creates_no_pull_request() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    // Update manifest origin to github URL
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                crate::store::manifest::Repo::new(
                    r.name().clone(),
                    "https://github.com/acme/api",
                    r.default_branch().clone(),
                )
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = crate::store::manifest::Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    crate::store::manifest::Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
        },
    )
    .expect("land preview");

    assert_eq!(
        land_preview.value.preview.repos[0].action,
        DeliveryAction::LandOnDefault
    );

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
        },
    )
    .expect("land apply");

    assert!(
        out.value.preview.repos[0].pr_url.is_none(),
        "land mode must not create a PR URL"
    );
}
