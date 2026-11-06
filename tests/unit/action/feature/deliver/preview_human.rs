use super::fixture::*;
use super::*;

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
            memory_review: MemoryReviewGate::Reviewed,
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
            memory_review: MemoryReviewGate::Reviewed,
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
            memory_review: MemoryReviewGate::Reviewed,
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
            memory_review: MemoryReviewGate::Reviewed,
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
fn the_human_preview_prints_the_apply_command_and_what_the_fingerprint_covers() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
            memory_review: MemoryReviewGate::Reviewed,
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
            memory_review: MemoryReviewGate::Reviewed,
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
