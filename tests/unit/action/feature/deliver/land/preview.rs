use super::super::fixture::*;
use super::super::*;

#[test]
fn push_and_land_previews_of_the_same_state_fingerprint_differently() {
    let feature = FeatureName::new("checkout").unwrap();
    let repos = vec![];
    let push = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &repos,
    )
    .expect("push fingerprint");
    let land = fingerprint_for(
        &feature,
        DeliveryMode::Land,
        GateState::Approved,
        &[],
        &repos,
    )
    .expect("land fingerprint");
    assert_ne!(
        push, land,
        "a push-approved fingerprint must not authorise a land"
    );
}

#[test]
fn a_push_fingerprint_cannot_be_applied_as_a_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let push_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: false,
            fingerprint: None,
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
        },
    )
    .expect("push preview");

    let refused = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(push_preview.value.preview.fingerprint.clone()),
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
        },
    );
    let failure = refused.expect_err("a push fingerprint must not open a land");
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
}

#[test]
fn a_land_fingerprint_cannot_be_applied_as_a_push() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

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

    let refused = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: false,
            fingerprint: Some(land_preview.value.preview.fingerprint.clone()),
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
        },
    );
    let failure = refused.expect_err("a land fingerprint must not open a push");
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
}

#[test]
fn land_preview_reports_ff_possible_without_touching_the_remote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let out = deliver(&ctx, land_preview_input("checkout")).expect("land preview");
    let repo = &out.value.preview.repos[0];
    assert_eq!(repo.ff_possible, Some(true));
    assert_eq!(repo.default_branch.as_ref().unwrap().as_str(), "main");
}

#[test]
fn a_feature_at_the_default_tip_is_fast_forwardable() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    let tip = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    git(&default_worktree, &["reset", "--hard", tip.trim()]);

    let out = deliver(&ctx, land_preview_input("checkout")).expect("land preview");
    assert_eq!(out.value.preview.repos[0].ff_possible, Some(true));
}

#[test]
fn land_preview_names_the_target_and_the_mode() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Land,
            plan_gate: GateState::Approved,
            repos: vec![DeliveryRepo {
                repo: RepoName::new("api").unwrap(),
                local_branch: BranchName::new("land-on-default").unwrap(),
                remote: "https://github.com/acme/api".to_owned(),
                push_refspec: "land-on-default:refs/heads/land-on-default".to_owned(),
                action: DeliveryAction::LandOnDefault,
                base_branch: BranchName::new("main").unwrap(),
                dependencies: Vec::new(),
                blockers: Vec::new(),
                pr_url: None,
                default_branch: Some(BranchName::new("main").unwrap()),
                ff_possible: Some(true),
                remote_default_tip: None,
                pr_title: None,
                pr_body: None,
            }],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("land on default"));
    assert!(rendered.contains("api  land-on-default -> main  fast-forward"));
    assert!(!rendered.contains("pull request"), "land opens no PR");

    let json_val = serde_json::to_value(&outcome.preview).unwrap();
    assert_eq!(json_val["mode"], "land");
    assert_eq!(json_val["repos"][0]["default_branch"], "main");
    assert_eq!(json_val["repos"][0]["ff_possible"], true);
}

#[test]
fn land_on_default_serialises_as_snake_case_and_has_a_word() {
    let action = DeliveryAction::LandOnDefault;
    assert_eq!(
        serde_json::to_value(action).unwrap(),
        serde_json::json!("land_on_default")
    );
    assert_eq!(outcome::action_word(action), "land on default");
}
