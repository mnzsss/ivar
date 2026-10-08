use super::fixture::*;
use super::*;

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pushes.is_empty(), "preview must not push");
    assert_eq!(report.value.preview.repos.len(), 1);
    let repo = &report.value.preview.repos[0];
    assert_eq!(repo.repo.as_str(), "api");
    assert_eq!(repo.local_branch.as_str(), "checkout");
    assert!(repo.remote.contains("origins/api"), "was: {}", repo.remote);
    assert_eq!(repo.push_refspec, "checkout:refs/heads/checkout");
    assert_eq!(repo.action, DeliveryAction::PushOnly);
    assert_eq!(repo.base_branch.as_str(), "main");
    assert!(repo.dependencies.is_empty());
    // One commit beyond main, no upstream: pending work, not a blocker.
    assert!(
        repo.pending
            .iter()
            .any(|pending| pending.contains("1 commit(s) not pushed")),
        "was: {:?}",
        repo.pending
    );
    assert!(repo.blockers.is_empty(), "was: {:?}", repo.blockers);
    // Preview is side-effect-free: the remote has no branch yet.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

/// `base_branch` in the preview is the base `promote` actually recorded —
/// the feature's declared base, not always the repo's default branch.
#[test]
fn preview_shows_the_recorded_base_not_always_the_default_branch() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    git(&origin, &["add", "develop-only.txt"]);
    git(&origin, &["commit", "-m", "develop work"]);
    git(&origin, &["checkout", "main"]);

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: Some("develop".to_owned()),
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert_eq!(
        report.value.preview.repos[0].base_branch.as_str(),
        "develop"
    );
}

#[test]
fn preview_fingerprint_changes_when_pr_metadata_changes() {
    let feature = FeatureName::new("checkout").unwrap();
    let mut repo = delivery_repo("api", Vec::new());
    let fp_none = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo.clone()],
    )
    .unwrap();

    repo.pr_title = Some("feat: something new".to_owned());
    let fp_title = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo.clone()],
    )
    .unwrap();

    repo.pr_body = Some("detailed body".to_owned());
    let fp_both = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo.clone()],
    )
    .unwrap();

    assert_ne!(fp_none, fp_title);
    assert_ne!(fp_title, fp_both);
    assert_ne!(fp_none, fp_both);
}

#[test]
fn the_preview_has_a_stable_content_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let first = deliver(&ctx, preview_input("checkout")).unwrap();
    let second = deliver(&ctx, preview_input("checkout")).unwrap();

    let fingerprint = &first.value.preview.fingerprint;
    assert_eq!(fingerprint.len(), 64, "a sha-256 hex digest");
    assert_eq!(fingerprint, &second.value.preview.fingerprint);
}

#[test]
fn a_feature_with_no_promoted_repos_previews_empty() {
    let (_guard, root) = hall_with_promoted(&[]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.value.preview.repos.is_empty());
    assert_eq!(report.value.preview.fingerprint.len(), 64);
}

#[test]
fn a_dirty_worktree_is_listed_as_a_blocker() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    let worktree = Layout::at(root.clone()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("notes.md"), "mine\n").unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert!(
        repo.blockers
            .iter()
            .any(|blocker| blocker.contains("uncommitted changes")),
        "was: {:?}",
        repo.blockers
    );
}

#[test]
fn delivering_a_missing_feature_is_blocked() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root);

    let failure = deliver(&ctx, preview_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

// -- apply: gating --------------------------------------------------------

#[test]
fn deliver_preview_reports_tree_blockers_and_apply_refuses_before_push() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    // An active leaf under the root blocks its delivery.
    child_of_checkout(&root, "child");
    let layout = Layout::at(root.clone());
    let mut leaf = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("leaf").unwrap(),
        BranchName::new("leaf").unwrap(),
    );
    leaf.parent = Some(crate::domain::name::FeatureName::new("child").unwrap());
    leaf.write(&layout).unwrap();

    // The preview fingerprints the blockers and reports them.
    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(report.value.preview.tree_blockers.len(), 2);
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(names, ["child", "leaf"]);
    assert_eq!(report.value.preview.tree_blockers[0].depth, 1);

    // Apply refuses before any push.
    let fingerprint = report.value.preview.fingerprint.clone();
    let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();
    assert_eq!(failure.code, "deliver.descendants_block");
    assert!(failure.actual.as_deref().unwrap().contains("child"));
}

#[test]
fn deliver_ignores_abandoned_descendants_but_sees_active_grandchildren() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    child_of_checkout(&root, "abandoned");
    let layout = Layout::at(&root);
    let mut grandchild = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("active").unwrap(),
        BranchName::new("active").unwrap(),
    );
    grandchild.parent = Some(crate::domain::name::FeatureName::new("abandoned").unwrap());
    grandchild.write(&layout).unwrap();
    crate::action::feature::lifecycle::write_close(
        &layout,
        &crate::domain::name::FeatureName::new("abandoned").unwrap(),
        crate::domain::feature::PromotionOutcome::Abandoned,
    )
    .unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(
        names,
        ["active"],
        "abandoned history does not block, but its active grandchild does"
    );
}

// -- rendering ------------------------------------------------------------

#[test]
fn the_human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: Vec::new(),
        apply_command: None,
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(rendered.contains("Delivery preview for `checkout` in /hall:"));
    assert!(rendered.contains("branch:  checkout"));
    assert!(rendered.contains("refspec: checkout:refs/heads/checkout"));
    assert!(rendered.contains("base:    main"));
    assert!(rendered.contains("action:  push only"));
    assert!(rendered.contains("blockers: none"));
    assert!(rendered.contains("fingerprint: abc123"));
}

#[test]
fn preview_without_mode_defaults_to_push() {
    let json = serde_json::json!({
        "feature": "checkout",
        "plan_gate": "approved",
        "repos": [],
        "fingerprint": ""
    });
    let preview: DeliveryPreview = serde_json::from_value(json).expect("legacy preview");
    assert_eq!(preview.mode, DeliveryMode::Push);
}

// -- fingerprint sensitivity -----------------------------------------------

#[test]
fn preview_fingerprint_changes_when_draft_action_differs() {
    let feature = FeatureName::new("checkout").unwrap();

    let mut repo_none = delivery_repo("api", Vec::new());
    let fp_none = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo_none.clone()],
    )
    .unwrap();

    repo_none.draft = Some(DraftAction::CreateAsDraft);
    let fp_draft = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo_none.clone()],
    )
    .unwrap();

    repo_none.draft = Some(DraftAction::ConvertToDraft);
    let fp_convert = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &[repo_none],
    )
    .unwrap();

    assert_ne!(
        fp_none, fp_draft,
        "draft intent must change the fingerprint"
    );
    assert_ne!(
        fp_draft, fp_convert,
        "create vs convert must differ in the fingerprint"
    );
    assert_ne!(
        fp_none, fp_convert,
        "convert action must change the fingerprint"
    );
}

// -- human rendering: draft actions ----------------------------------------

#[test]
fn human_preview_renders_new_pr_draft() {
    let mut repo = delivery_repo("api", Vec::new());
    repo.action = DeliveryAction::NewPr;
    repo.draft = Some(DraftAction::CreateAsDraft);
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![repo],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: Vec::new(),
        apply_command: None,
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let rendered = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(
        rendered.contains("action:  new pr (draft)"),
        "expected 'action:  new pr (draft)' in:\n{rendered}"
    );
}

#[test]
fn human_preview_renders_convert_pr_to_draft() {
    let mut repo = delivery_repo("api", Vec::new());
    repo.action = DeliveryAction::UpdatePr;
    repo.draft = Some(DraftAction::ConvertToDraft);
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![repo],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: Vec::new(),
        apply_command: None,
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let rendered = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(
        rendered.contains("action:  update pr"),
        "expected 'action:  update pr' in:\n{rendered}"
    );
    assert!(
        rendered.contains("action:  convert pr to draft"),
        "expected 'action:  convert pr to draft' in:\n{rendered}"
    );
    // The convert line must follow the update line.
    let update_pos = rendered.find("action:  update pr").unwrap();
    let convert_pos = rendered.find("action:  convert pr to draft").unwrap();
    assert!(
        update_pos < convert_pos,
        "update pr must appear before convert pr to draft"
    );
}

#[test]
fn human_preview_without_draft_omits_draft_text() {
    let repo = delivery_repo("api", Vec::new());
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![repo],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: Vec::new(),
        apply_command: None,
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let rendered = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(
        rendered.contains("action:  push only"),
        "expected 'action:  push only' in:\n{rendered}"
    );
    assert!(
        !rendered.contains("draft"),
        "draft must not appear in legacy output:\n{rendered}"
    );
}

#[test]
fn apply_command_repeats_every_fingerprinted_flag() {
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: Some("feat: checkout flow".to_owned()),
            body: Some("./docs/pr.md".to_owned()),
            draft: Some(true),
        },
        repo_overrides: vec![RepoMetadataOverride {
            repo: "api".to_owned(),
            metadata: PullRequestMetadata {
                title: Some("it's the api".to_owned()),
                body: None,
                draft: None,
            },
        }],
        only: Vec::new(),
    };

    assert_eq!(
        apply_command(&input, "abc123"),
        "ivar feature deliver checkout --fingerprint abc123 \
         --name 'feat: checkout flow' --body ./docs/pr.md --draft \
         --repo api --name 'it'\\''s the api'"
    );
}

#[test]
fn the_human_preview_prints_the_apply_command_and_what_the_fingerprint_covers() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: Vec::new(),
        apply_command: Some("ivar feature deliver checkout --fingerprint abc123".to_owned()),
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(rendered.contains("fingerprint: abc123"));
    assert!(rendered.contains("apply:       ivar feature deliver checkout --fingerprint abc123"));
    assert!(rendered.contains("--name, --body, --draft and --only are part of the fingerprint"));
}

#[test]
fn preview_mode_carries_the_apply_command_and_land_flag() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let preview = deliver(&ctx, land_preview_input("checkout")).unwrap().value;

    assert_eq!(
        preview.apply_command,
        Some(format!(
            "ivar feature deliver checkout --land --fingerprint {}",
            preview.preview.fingerprint
        ))
    );
}

#[test]
fn apply_command_repeats_every_only_selection() {
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        only: vec!["api".to_owned(), "web".to_owned()],
        ..Default::default()
    };

    assert_eq!(
        apply_command(&input, "abc123"),
        "ivar feature deliver checkout --only api --only web --fingerprint abc123"
    );
}

#[test]
fn only_restricts_the_preview_to_the_selected_repos() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let report = deliver(
        &ctx,
        DeliverInput {
            only: vec!["web".to_owned()],
            ..preview_input("checkout")
        },
    )
    .unwrap();

    let repos: Vec<&str> = report
        .value
        .preview
        .repos
        .iter()
        .map(|r| r.repo.as_str())
        .collect();
    assert_eq!(repos, vec!["web"]);
    assert!(report.value.apply_command.unwrap().contains("--only web"));
}

#[test]
fn a_different_selection_fingerprints_differently() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let fingerprint = |only: &[&str]| {
        deliver(
            &ctx,
            DeliverInput {
                only: only.iter().map(|r| (*r).to_owned()).collect(),
                ..preview_input("checkout")
            },
        )
        .unwrap()
        .value
        .preview
        .fingerprint
    };

    assert_ne!(fingerprint(&["api"]), fingerprint(&["web"]));
    assert_ne!(fingerprint(&["api"]), fingerprint(&[]));
    assert_eq!(fingerprint(&["api", "web"]), fingerprint(&[]));
}

#[test]
fn the_human_preview_paints_only_its_labels() {
    use crate::error::{CAUTION, HEADER, MUTED, paint};

    let mut blocked = delivery_repo("api", vec![]);
    blocked.pending = vec!["1 commit(s) not pushed".to_owned()];
    blocked.blockers = vec!["worktree is dirty".to_owned()];
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![blocked, delivery_repo("web", vec![])],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        blockers: vec!["plan gate is pending".to_owned()],
        apply_command: Some("ivar feature deliver checkout --fingerprint abc123".to_owned()),
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();

    for label in [
        "branch:",
        "remote:",
        "refspec:",
        "base:",
        "action:",
        "pending:",
        "blockers:",
        "plan gate:",
        "fingerprint:",
        "apply:",
        "note:",
    ] {
        assert!(
            rendered.contains(&paint(MUTED, label)),
            "{label} must be painted MUTED in:\n{rendered}"
        );
    }
    for label in ["blocker:", "blocked:"] {
        assert!(
            rendered.contains(&paint(CAUTION, label)),
            "{label} must be painted CAUTION in:\n{rendered}"
        );
    }
    for repo in ["api:", "web:"] {
        assert!(
            rendered.contains(&paint(HEADER, repo)),
            "{repo} must be painted HEADER in:\n{rendered}"
        );
    }
    for line in rendered.lines() {
        let value = line.rsplit("\x1b[0m").next().unwrap_or(line);
        assert!(!value.contains('\x1b'), "a value is painted: {line:?}");
    }

    let plain = concat!(
        "Delivery preview for `checkout` in /hall:\n",
        "  api:\n",
        "    branch:  checkout\n",
        "    remote:  git@example.com:acme/api.git\n",
        "    refspec: checkout:refs/heads/checkout\n",
        "    base:    main\n",
        "    action:  push only\n",
        "    pending: 1 commit(s) not pushed\n",
        "    blocker: worktree is dirty\n",
        "  web:\n",
        "    branch:  checkout\n",
        "    remote:  git@example.com:acme/api.git\n",
        "    refspec: checkout:refs/heads/checkout\n",
        "    base:    main\n",
        "    action:  push only\n",
        "    blockers: none\n",
        "  plan gate:   approved\n",
        "  blocked:     plan gate is pending\n",
        "  fingerprint: abc123\n",
        "  apply:       ivar feature deliver checkout --fingerprint abc123\n",
        "  note:        --name, --body, --draft and --only are part of the fingerprint; apply with the same values\n",
    );
    assert_eq!(anstream::adapter::strip_str(&rendered).to_string(), plain);
}

#[test]
fn human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let mut out = Vec::new();
    deliver(&Ctx::new(root.clone()), preview_input("checkout"))
        .unwrap()
        .value
        .write_human(&mut out)
        .unwrap();

    let human = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(human.contains("Delivery preview for `checkout`"));
    assert!(human.contains("branch:  checkout"));
    assert!(human.contains("refspec: checkout:refs/heads/checkout"));
    assert!(human.contains("base:    main"));
    // The remote is a local path — push only, no PR.
    assert!(human.contains("action:  push only"));
    assert!(human.contains("fingerprint:"));
}

#[test]
fn the_preview_reports_the_plan_gate_without_refusing() {
    let (_guard, root) = hall_with_promoted(&["api"]);

    let value = preview_json(&root, preview_input("checkout"));
    assert_eq!(value["preview"]["plan_gate"], "pending");

    approve_through_plan(&root);

    let value = preview_json(&root, preview_input("checkout"));
    assert_eq!(value["preview"]["plan_gate"], "approved");
}
