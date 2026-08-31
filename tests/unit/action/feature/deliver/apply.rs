use super::fixture::*;
use super::*;

#[test]
fn apply_requires_a_preview_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: false,
            fingerprint: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "deliver.preview_required");
}

#[test]
fn apply_is_rejected_when_the_state_has_drifted_since_the_preview() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let fingerprint = approved.value.preview.fingerprint.clone();

    // Drift: one more commit lands on the feature branch.
    let worktree = Layout::at(root.clone()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("more.md"), "more\n").unwrap();
    git(&worktree, &["add", "more.md"]);
    git(&worktree, &["commit", "-m", "more"]);

    let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "deliver.re_preview"),
        "the fix must re-run the preview: {:?}",
        failure.fix_actions
    );
    // Nothing was pushed.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

// -- apply: pushing -------------------------------------------------------

#[test]
fn the_human_apply_surface_reports_each_push() {
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
        pushes: vec![
            PushResult {
                repo: RepoName::new("api").unwrap(),
                ok: true,
                detail: None,
            },
            PushResult {
                repo: RepoName::new("web").unwrap(),
                ok: false,
                detail: Some("remote did not answer".to_owned()),
            },
        ],
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("Delivered `checkout` in /hall (fingerprint abc123):"));
    assert!(rendered.contains("  api: pushed"));
    assert!(rendered.contains("  web: not pushed — remote did not answer"));
}

/// The short path's sharp edge, closed at the surface that matters. A feature
/// that approved `plan` while `requirements.md` did not exist may deliver.
/// Once `requirements.md` appears unapproved, that approval no longer holds,
/// and `deliver` has to refuse rather than ship on a gate `plan approve` would
/// now decline to grant.
#[test]
fn deliver_refuses_once_an_upstream_artifact_appears_after_approval() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    // Short path: scaffold and approve the plan gate alone.
    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![crate::action::plan::Artifact::Plan],
        },
    )
    .unwrap();
    crate::action::plan::approve::approve(
        &ctx,
        crate::action::plan::approve::ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        deliver(&ctx, preview_input("checkout"))
            .unwrap()
            .value
            .preview
            .plan_gate,
        GateState::Approved
    );

    // The upstream artifact appears, unapproved.
    crate::infra::fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n",
    )
    .unwrap();

    let preview = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(preview.value.preview.plan_gate, GateState::NeedsRevision);

    let failure = deliver(
        &ctx,
        apply_input("checkout", &preview.value.preview.fingerprint),
    )
    .unwrap_err();
    assert_eq!(failure.code, "deliver.plan_not_approved");
}

/// `deliver` read `approvals.json` raw, which answers a question about the
/// last command that wrote it rather than about the feature as it stands. A
/// plan.md edited after approval was reported `needs-revision` by
/// `ivar plan status` and shipped by `deliver` anyway — the tool enforcing one
/// rule and reporting another.
#[test]
fn deliver_refuses_a_plan_edited_after_it_was_approved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    assert_eq!(
        deliver(&ctx, preview_input("checkout"))
            .unwrap()
            .value
            .preview
            .plan_gate,
        GateState::Approved
    );

    // A human rewrites the plan after approving it.
    let plan_path = root.join("plans/checkout/plan.md");
    let body = crate::infra::fs::read_text(&plan_path).unwrap().unwrap();
    crate::infra::fs::write_text(&plan_path, &format!("{body}\nrewritten\n")).unwrap();

    let preview = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(preview.value.preview.plan_gate, GateState::NeedsRevision);

    let failure = deliver(
        &ctx,
        apply_input("checkout", &preview.value.preview.fingerprint),
    )
    .unwrap_err();
    assert_eq!(failure.code, "deliver.plan_not_approved");
}
