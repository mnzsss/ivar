//! Unit tests for `crate::domain::feature::run` — the Run Receipt aggregate,
//! its status machine, and the coordinator report it accepts as evidence.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
//!
//! Every test here is a pure value comparison. That is not an accident of
//! style: [`RunReceipt`] takes its id, its clock and its baseline from the
//! caller precisely so the state machine can be exercised without a temp
//! directory, a uuid, or a git repository anywhere in sight.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

// -- fixtures ---------------------------------------------------------------

/// A stable run id. A literal rather than a fresh uuid so a failure names the
/// same value every run.
fn run_id() -> RunId {
    RunId::new("6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33").unwrap()
}

fn feature() -> FeatureName {
    FeatureName::new("checkout").unwrap()
}

fn session(tail: &str) -> SessionId {
    SessionId::new(format!("11111111-2222-3333-4444-5555555555{tail}")).unwrap()
}

/// An active run at `T0`, authorised against `plan-fp-1`.
fn started() -> RunReceipt {
    RunReceipt::start(
        run_id(),
        feature(),
        "plans/checkout/plan.md",
        "plan-fp-1",
        RunBaseline::empty(),
        session("01"),
        Provider::ClaudeCode,
        "2026-08-14T00:00:00Z",
    )
}

/// The smallest report [`CoordinatorReport::validate`] accepts.
fn report() -> CoordinatorReport {
    CoordinatorReport {
        summary: "wired the receipt store".to_owned(),
        tasks: vec![TaskResult {
            title: "add run.json persistence".to_owned(),
            status: TaskStatus::Completed,
            result: "current receipt and archive round-trip".to_owned(),
        }],
        verification: vec![VerificationCheck {
            command: "cargo test".to_owned(),
            status: CheckStatus::Passed,
            summary: "all green".to_owned(),
        }],
        agents: Vec::new(),
        deviations: Vec::new(),
        blockers: Vec::new(),
        follow_ups: Vec::new(),
    }
}

/// A diff naming one added path in one repo — enough to prove evidence is
/// carried onto a checkpoint, without pretending to be a real snapshot.
fn diff() -> RunDiff {
    let mut changes = BTreeMap::new();
    changes.insert(
        Utf8PathBuf::from("src/store/feature/run.rs"),
        PathChange {
            kind: ChangeKind::Added,
            final_state: PathEvidence::file(0o100_644, "hash-a"),
        },
    );
    let mut repos = BTreeMap::new();
    repos.insert(
        "ivar".to_owned(),
        RepoDiff {
            head: "c0ffee".to_owned(),
            changes,
        },
    );
    RunDiff { repos }
}

fn legacy_evidence() -> LegacyEvidence {
    LegacyEvidence {
        source_hash: "board-hash".to_owned(),
        board_status: "running".to_owned(),
        plan_fingerprint: Some("plan-fp-old".to_owned()),
        workstreams: vec![LegacyWorkstream {
            id: "receipt-core".to_owned(),
            title: "Run Receipt domain".to_owned(),
            status: "active".to_owned(),
            operations: vec!["OP-RUN-DOMAIN".to_owned()],
            depends_on: Vec::new(),
        }],
        sessions: BTreeMap::new(),
        journal: Vec::new(),
        archived_board: Utf8PathBuf::from("archive/boards/board-hash.json"),
    }
}

// -- run id -----------------------------------------------------------------

/// The id is a path component. A value that could hold `..` or `/` would turn
/// `status --run <id>` into a traversal, so validation is the constructor.
#[test]
fn a_run_id_must_be_a_uuid() {
    assert!(RunId::new("6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33").is_ok());
    for rejected in ["", "../../etc/passwd", "run-1", "6f1d9e64/0d1a"] {
        assert!(
            RunId::new(rejected).is_err(),
            "`{rejected}` must not parse as a run id"
        );
    }
}

/// Deserialization routes through the same constructor, so a hand-edited
/// `run.json` cannot smuggle a traversal past the type.
#[test]
fn deserializing_a_run_id_validates_it_too() {
    assert!(serde_json::from_str::<RunId>(r#""6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33""#).is_ok());
    assert!(serde_json::from_str::<RunId>(r#""../escape""#).is_err());
}

#[test]
fn an_invalid_run_id_failure_points_at_the_history() {
    let failure = Failure::from(InvalidRunId("nope".to_owned()));

    assert_eq!(failure.code, "execute.invalid_run_id");
    assert_eq!(failure.fix_actions.len(), 1);
}

// -- status ----------------------------------------------------------------

/// The whole state machine reduces to this split: three states hold the
/// feature's single-run lock and three release it.
#[test]
fn the_three_non_terminal_states_hold_the_lock_and_the_three_terminal_ones_do_not() {
    for live in [RunStatus::Active, RunStatus::Blocked, RunStatus::Diverged] {
        assert!(live.holds_lock(), "{live} must hold the run lock");
        assert!(!live.is_terminal(), "{live} must not be terminal");
    }
    for over in [
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Interrupted,
    ] {
        assert!(over.is_terminal(), "{over} must be terminal");
        assert!(!over.holds_lock(), "{over} must release the run lock");
    }
}

#[test]
fn an_outcome_names_the_status_it_produces() {
    assert_eq!(RunOutcome::Succeeded.status(), RunStatus::Succeeded);
    assert_eq!(RunOutcome::Failed.status(), RunStatus::Failed);
    assert_eq!(RunOutcome::Blocked.status(), RunStatus::Blocked);
}

#[test]
fn outcomes_parse_from_their_cli_spelling_and_nothing_else() {
    assert_eq!(RunOutcome::parse("succeeded"), Ok(RunOutcome::Succeeded));
    assert_eq!(RunOutcome::parse("failed"), Ok(RunOutcome::Failed));
    assert_eq!(RunOutcome::parse("blocked"), Ok(RunOutcome::Blocked));
    assert_eq!(
        RunOutcome::parse("done"),
        Err(UnknownRunOutcome("done".to_owned()))
    );
}

/// Display is the CLI spelling for both, and both go through `pad` so the
/// status column in `execute status` actually aligns.
#[test]
fn statuses_and_provenance_render_as_their_cli_spelling() {
    assert_eq!(RunStatus::Diverged.to_string(), "diverged");
    assert_eq!(RunProvenance::Native.to_string(), "native");
    assert_eq!(RunProvenance::LegacyImport.to_string(), "legacy-import");
    assert_eq!(format!("{:<12}|", RunStatus::Active), "active      |");
}

// -- path evidence and classification --------------------------------------

#[test]
fn path_evidence_knows_whether_anything_is_there() {
    assert!(!PathEvidence::absent().exists());
    assert!(PathEvidence::file(0o100_644, "h").exists());
    assert!(PathEvidence::symlink("h").exists());
    assert_eq!(PathEvidence::symlink("h").mode, Some(0o120_000));
}

/// A path the run did not touch must not appear in the diff at all — which is
/// how inherited dirty work that nobody edited escapes being blamed on the
/// run.
#[test]
fn an_unchanged_path_classifies_as_no_change_at_all() {
    let same = PathEvidence::file(0o100_644, "hash-a");
    let commit = PathEvidence::file(0o100_644, "hash-committed");

    assert_eq!(classify_change(&same, &commit, &same.clone()), None);
}

#[test]
fn added_removed_and_modified_come_from_the_two_boundaries() {
    let absent = PathEvidence::absent();
    let a = PathEvidence::file(0o100_644, "hash-a");
    let b = PathEvidence::file(0o100_644, "hash-b");

    assert_eq!(
        classify_change(&absent, &absent, &a),
        Some(ChangeKind::Added)
    );
    assert_eq!(classify_change(&a, &a, &absent), Some(ChangeKind::Removed));
    assert_eq!(classify_change(&a, &a, &b), Some(ChangeKind::Modified));
}

/// Flipping the executable bit changes no content hash, which is exactly why
/// the mode is recorded next to it.
#[test]
fn a_mode_change_alone_is_a_modification() {
    let before = PathEvidence::file(0o100_644, "hash-a");
    let after = PathEvidence::file(0o100_755, "hash-a");

    assert_eq!(
        classify_change(&before, &before, &after),
        Some(ChangeKind::Modified)
    );
}

/// Inherited dirty work the run undid. Neither "modified into something new"
/// nor harmless — it destroyed work the run did not create, so it gets its
/// own name.
#[test]
fn a_path_dragged_back_to_the_starting_commit_is_reverted() {
    let commit = PathEvidence::file(0o100_644, "hash-committed");
    let dirty = PathEvidence::file(0o100_644, "hash-dirty");

    assert_eq!(
        classify_change(&dirty, &commit, &commit),
        Some(ChangeKind::Reverted)
    );
}

/// The narrow reading of `Reverted`: the path must have *diverged* at start.
/// A clean file edited and edited back is no change at all.
#[test]
fn a_clean_path_edited_and_edited_back_is_not_reverted() {
    let commit = PathEvidence::file(0o100_644, "hash-committed");

    assert_eq!(classify_change(&commit, &commit, &commit.clone()), None);
}

/// A symlink that now points somewhere else is a real edit that a
/// file-content comparison alone would miss.
#[test]
fn a_symlink_retargeted_is_a_modification() {
    let before = PathEvidence::symlink("target-a");
    let after = PathEvidence::symlink("target-b");

    assert_eq!(
        classify_change(&before, &before, &after),
        Some(ChangeKind::Modified)
    );
}

/// A file replaced by a symlink shares neither state nor hash, and must not
/// read as unchanged just because both exist.
#[test]
fn a_file_replaced_by_a_symlink_is_a_modification() {
    let before = PathEvidence::file(0o100_644, "hash-a");
    let after = PathEvidence::symlink("hash-a");

    assert_eq!(
        classify_change(&before, &before, &after),
        Some(ChangeKind::Modified)
    );
}

#[test]
fn a_repo_diff_can_list_the_paths_of_one_kind() {
    let diff = diff();
    let repo = &diff.repos["ivar"];

    assert_eq!(
        repo.of_kind(ChangeKind::Added).collect::<Vec<_>>(),
        vec![&Utf8PathBuf::from("src/store/feature/run.rs")]
    );
    assert_eq!(repo.of_kind(ChangeKind::Removed).count(), 0);
    assert!(!diff.is_empty());
    assert!(RunDiff::default().is_empty());
}

// -- coordinator report -----------------------------------------------------

#[test]
fn a_complete_report_validates() {
    assert!(report().validate().is_ok());
}

#[test]
fn a_report_with_a_blank_summary_is_refused() {
    let mut report = report();
    report.summary = "   \n".to_owned();

    let failure = report.validate().unwrap_err();

    assert_eq!(failure.code, "execute.report_summary_blank");
    assert_eq!(failure.fix_actions.len(), 1);
}

#[test]
fn a_report_with_no_tasks_is_refused() {
    let mut report = report();
    report.tasks.clear();

    assert_eq!(
        report.validate().unwrap_err().code,
        "execute.report_no_tasks"
    );
}

/// A run that verified nothing has not finished, it has stopped.
#[test]
fn a_report_with_no_verification_is_refused() {
    let mut report = report();
    report.verification.clear();

    assert_eq!(
        report.validate().unwrap_err().code,
        "execute.report_no_verification"
    );
}

/// The three refusals are separate because "your report is invalid" is not an
/// actionable sentence — each one names the field to fill.
#[test]
fn each_missing_report_field_gets_its_own_refusal() {
    let mut blank = report();
    blank.summary = String::new();
    blank.tasks.clear();
    blank.verification.clear();

    // Summary first: it is the field a hurrying coordinator drops first.
    assert_eq!(
        blank.validate().unwrap_err().code,
        "execute.report_summary_blank"
    );
}

/// `deny_unknown_fields` is load-bearing rather than tidy. This is what stops
/// a provider envelope from becoming ivar domain state by accident.
#[test]
fn a_report_carrying_an_unknown_field_does_not_deserialize() {
    let error = serde_json::from_str::<CoordinatorReport>(
        r#"{
            "summary": "done",
            "tasks": [{"title": "t", "status": "completed", "result": "r"}],
            "verification": [{"command": "cargo test", "status": "passed", "summary": "ok"}],
            "notes": "smuggled"
        }"#,
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("notes"),
        "the refusal should name the unknown field: {error}"
    );
}

/// The specific unknown field this feature exists to keep out: an opaque
/// provider-native conversation id. Ivar records its own session lineage and
/// claims nothing about a vendor's transcript.
#[test]
fn a_report_carrying_a_provider_native_identifier_does_not_deserialize() {
    for smuggled in [
        r#""session_id": "b8d74d19-7e1b-40cb-892a-d677c454f335""#,
        r#""native_session_id": "abc""#,
        r#""transcript_path": "/tmp/x.jsonl""#,
    ] {
        let json = format!(
            r#"{{
                "summary": "done",
                "tasks": [{{"title": "t", "status": "completed", "result": "r"}}],
                "verification": [{{"command": "c", "status": "passed", "summary": "ok"}}],
                {smuggled}
            }}"#
        );
        assert!(
            serde_json::from_str::<CoordinatorReport>(&json).is_err(),
            "a report must not accept {smuggled}"
        );
    }
}

/// A subagent is described by role and status, never by a native child id —
/// the identifier is provider-specific, unstable, and worthless to whoever
/// reads the receipt later.
#[test]
fn an_agent_role_carries_no_native_identity() {
    assert!(
        serde_json::from_str::<AgentRole>(r#"{"role": "reviewer", "status": "completed"}"#).is_ok()
    );
    assert!(
        serde_json::from_str::<AgentRole>(
            r#"{"role": "reviewer", "status": "completed", "child_id": "sub_01"}"#
        )
        .is_err()
    );
}

/// The optional halves round-trip through the skip-if-empty attributes
/// without gaining or losing a field.
#[test]
fn a_report_round_trips_through_json() {
    let mut original = report();
    original.agents.push(AgentRole {
        role: "reviewer".to_owned(),
        status: TaskStatus::Completed,
    });
    original.deviations.push("skipped the docs pass".to_owned());
    original.blockers.push("needs a human answer".to_owned());
    original
        .follow_ups
        .push("extract the snapshot service".to_owned());

    let text = serde_json::to_string(&original).unwrap();
    let read_back: CoordinatorReport = serde_json::from_str(&text).unwrap();

    assert_eq!(read_back, original);
}

// -- start ------------------------------------------------------------------

#[test]
fn a_started_run_is_active_native_and_pinned_to_its_plan() {
    let receipt = started();

    assert_eq!(receipt.version, RUN_CURRENT_VERSION);
    assert_eq!(receipt.status, RunStatus::Active);
    assert_eq!(receipt.provenance, RunProvenance::Native);
    assert_eq!(receipt.plan_fingerprint, "plan-fp-1");
    assert_eq!(receipt.plan_path, "plans/checkout/plan.md");
    assert_eq!(receipt.started_at, "2026-08-14T00:00:00Z");
    assert_eq!(receipt.updated_at, "2026-08-14T00:00:00Z");
    assert_eq!(receipt.terminated_at, None);
    assert_eq!(receipt.outcome, None);
    assert_eq!(receipt.final_diff, None);
    assert_eq!(receipt.legacy, None);
    assert!(receipt.holds_lock());
}

#[test]
fn starting_records_the_first_coordinator_and_a_started_checkpoint() {
    let receipt = started();

    assert_eq!(receipt.coordinators.len(), 1);
    assert_eq!(receipt.coordinators[0].session, session("01"));
    assert_eq!(receipt.coordinators[0].provider, Provider::ClaudeCode);
    assert_eq!(receipt.checkpoints.len(), 1);
    assert_eq!(receipt.checkpoints[0].kind, CheckpointKind::Started);
    assert_eq!(receipt.checkpoints[0].status, RunStatus::Active);
    assert_eq!(
        receipt.checkpoints[0].plan_fingerprint_to.as_deref(),
        Some("plan-fp-1")
    );
}

// -- resume -----------------------------------------------------------------

#[test]
fn resuming_a_blocked_run_makes_it_active_again() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    receipt
        .resume(session("02"), Provider::OpenCode, "T2")
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Active);
    assert_eq!(receipt.updated_at, "T2");
    assert_eq!(
        receipt.checkpoints.last().unwrap().kind,
        CheckpointKind::Resumed
    );
}

/// A coordinator whose session died mid-run re-attaches without first having
/// to terminalize a run that was never finished.
#[test]
fn resuming_an_active_run_is_allowed() {
    let mut receipt = started();

    assert!(
        receipt
            .resume(session("02"), Provider::ClaudeCode, "T1")
            .is_ok()
    );
    assert_eq!(receipt.status, RunStatus::Active);
}

/// The plan moved. Adopting the new revision is `accept-revision`'s explicit
/// decision, never a side effect of re-attaching.
#[test]
fn resuming_a_diverged_run_is_refused() {
    let mut receipt = started();
    receipt
        .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    let refusal = receipt
        .resume(session("02"), Provider::ClaudeCode, "T2")
        .unwrap_err();

    assert_eq!(
        refusal,
        RunTransition::WrongState {
            status: RunStatus::Diverged,
            operation: "resume",
        }
    );
    assert_eq!(receipt.status, RunStatus::Diverged);
}

/// The point of recording lineage rather than a provider conversation id: a
/// run may begin under Claude Code and continue under OpenCode, and the
/// honest claim is two ordered entries, not continuity of a vendor thread.
#[test]
fn cross_provider_lineage_preserves_order() {
    let mut receipt = started();
    receipt
        .resume(session("02"), Provider::OpenCode, "T1")
        .unwrap();
    receipt
        .resume(session("03"), Provider::ClaudeCode, "T2")
        .unwrap();

    let lineage: Vec<_> = receipt
        .coordinators
        .iter()
        .map(|entry| {
            (
                entry.session.to_string(),
                entry.provider,
                entry.attached_at.clone(),
            )
        })
        .collect();

    assert_eq!(
        lineage,
        vec![
            (
                session("01").to_string(),
                Provider::ClaudeCode,
                "2026-08-14T00:00:00Z".to_owned()
            ),
            (
                session("02").to_string(),
                Provider::OpenCode,
                "T1".to_owned()
            ),
            (
                session("03").to_string(),
                Provider::ClaudeCode,
                "T2".to_owned()
            ),
        ]
    );
    assert_eq!(
        receipt.current_coordinator().unwrap().session,
        session("03")
    );
}

/// A receipt should record that a coordinator re-attached. Collapsing repeats
/// would lose exactly that.
#[test]
fn re_attaching_the_same_session_still_appends_a_lineage_entry() {
    let mut receipt = started();

    receipt
        .resume(session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    assert_eq!(receipt.coordinators.len(), 2);
    assert_eq!(
        receipt.coordinators[0].session,
        receipt.coordinators[1].session
    );
}

// -- block ------------------------------------------------------------------

/// Blocked keeps the run id, the baseline and the lock, because the run is
/// not over — and its evidence lands on a checkpoint, not on `final_diff`.
#[test]
fn blocking_keeps_the_lock_and_puts_its_evidence_on_a_checkpoint() {
    let mut receipt = started();

    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Blocked);
    assert!(receipt.holds_lock());
    assert_eq!(receipt.terminated_at, None);
    assert_eq!(receipt.final_diff, None);
    assert_eq!(receipt.outcome, None);

    let checkpoint = receipt.checkpoints.last().unwrap();
    assert_eq!(checkpoint.kind, CheckpointKind::Blocked);
    assert_eq!(checkpoint.report.as_ref(), Some(&report()));
    assert_eq!(checkpoint.diff.as_ref(), Some(&diff()));
}

#[test]
fn blocking_an_already_blocked_run_is_refused() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    let refusal = receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T2")
        .unwrap_err();

    assert_eq!(
        refusal,
        RunTransition::WrongState {
            status: RunStatus::Blocked,
            operation: "block",
        }
    );
}

// -- diverge ----------------------------------------------------------------

/// The coordinator's work is evidence whether or not its authorisation still
/// holds — so the report is kept, no outcome is accepted, and the pinned
/// fingerprint is *not* rewritten.
#[test]
fn diverging_preserves_the_submitted_report_without_re_authorising() {
    let mut receipt = started();

    receipt
        .diverge(
            "plan-fp-2",
            Some(report()),
            session("01"),
            Provider::ClaudeCode,
            "T1",
        )
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Diverged);
    assert!(receipt.holds_lock());
    assert_eq!(receipt.plan_fingerprint, "plan-fp-1");
    assert_eq!(receipt.outcome, None);

    let checkpoint = receipt.checkpoints.last().unwrap();
    assert_eq!(checkpoint.kind, CheckpointKind::Diverged);
    assert_eq!(checkpoint.report.as_ref(), Some(&report()));
    assert_eq!(
        checkpoint.plan_fingerprint_from.as_deref(),
        Some("plan-fp-1")
    );
    assert_eq!(checkpoint.plan_fingerprint_to.as_deref(), Some("plan-fp-2"));
}

#[test]
fn diverging_from_anything_but_active_is_refused() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    assert_eq!(
        receipt
            .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T2")
            .unwrap_err(),
        RunTransition::WrongState {
            status: RunStatus::Blocked,
            operation: "diverge",
        }
    );
}

// -- accept revision --------------------------------------------------------

/// Lands on `Blocked`, never straight on `Active`: attaching a coordinator is
/// `start --resume`'s job, and collapsing the two would let a revision be
/// accepted by a session that then never picks the work up.
#[test]
fn accepting_a_revision_repins_the_plan_and_lands_on_blocked() {
    let mut receipt = started();
    receipt
        .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    receipt
        .accept_revision("plan-fp-2", session("02"), Provider::ClaudeCode, "T2")
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Blocked);
    assert_eq!(receipt.plan_fingerprint, "plan-fp-2");

    let checkpoint = receipt.checkpoints.last().unwrap();
    assert_eq!(checkpoint.kind, CheckpointKind::RevisionAccepted);
    assert_eq!(
        checkpoint.plan_fingerprint_from.as_deref(),
        Some("plan-fp-1")
    );
    assert_eq!(checkpoint.plan_fingerprint_to.as_deref(), Some("plan-fp-2"));
}

#[test]
fn an_accepted_revision_can_then_be_resumed() {
    let mut receipt = started();
    receipt
        .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T1")
        .unwrap();
    receipt
        .accept_revision("plan-fp-2", session("01"), Provider::ClaudeCode, "T2")
        .unwrap();

    receipt
        .resume(session("03"), Provider::OpenCode, "T3")
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Active);
}

/// "Nothing changed" means the caller is looking at a different file than
/// finish was — the receipt says the plan diverged.
#[test]
fn accepting_the_fingerprint_already_pinned_is_refused() {
    let mut receipt = started();
    receipt
        .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    let refusal = receipt
        .accept_revision("plan-fp-1", session("01"), Provider::ClaudeCode, "T2")
        .unwrap_err();

    assert_eq!(
        refusal,
        RunTransition::RevisionUnchanged {
            fingerprint: "plan-fp-1".to_owned()
        }
    );
    assert_eq!(receipt.status, RunStatus::Diverged);
}

#[test]
fn accepting_a_revision_on_a_run_that_did_not_diverge_is_refused() {
    let mut receipt = started();

    assert_eq!(
        receipt
            .accept_revision("plan-fp-2", session("01"), Provider::ClaudeCode, "T1")
            .unwrap_err(),
        RunTransition::WrongState {
            status: RunStatus::Active,
            operation: "accept-revision",
        }
    );
}

// -- terminate --------------------------------------------------------------

#[test]
fn finishing_succeeded_terminalizes_with_its_final_evidence() {
    let mut receipt = started();

    receipt
        .terminate(
            RunOutcome::Succeeded,
            report(),
            diff(),
            session("01"),
            Provider::ClaudeCode,
            "T1",
        )
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Succeeded);
    assert_eq!(receipt.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(receipt.final_diff.as_ref(), Some(&diff()));
    assert_eq!(receipt.terminated_at.as_deref(), Some("T1"));
    assert!(!receipt.holds_lock());
    assert_eq!(
        receipt.checkpoints.last().unwrap().kind,
        CheckpointKind::Terminated
    );
}

#[test]
fn finishing_failed_terminalizes_too() {
    let mut receipt = started();

    receipt
        .terminate(
            RunOutcome::Failed,
            report(),
            diff(),
            session("01"),
            Provider::ClaudeCode,
            "T1",
        )
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Failed);
    assert_eq!(receipt.outcome, Some(RunOutcome::Failed));
    assert!(!receipt.holds_lock());
}

/// A blocked outcome is recoverable, so routing it through the terminal path
/// is a caller bug — refused rather than quietly redirected to `block`.
#[test]
fn terminating_with_a_blocked_outcome_is_refused() {
    let mut receipt = started();

    let refusal = receipt
        .terminate(
            RunOutcome::Blocked,
            report(),
            diff(),
            session("01"),
            Provider::ClaudeCode,
            "T1",
        )
        .unwrap_err();

    assert_eq!(refusal, RunTransition::BlockedIsNotTerminal);
    assert_eq!(receipt.status, RunStatus::Active);
}

/// A blocked run must be resumed before it can be finished. Terminating
/// straight from `blocked` would let a coordinator that never re-attached
/// close a run someone else was answering.
#[test]
fn terminating_a_blocked_run_without_resuming_is_refused() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    assert_eq!(
        receipt
            .terminate(
                RunOutcome::Succeeded,
                report(),
                diff(),
                session("01"),
                Provider::ClaudeCode,
                "T2",
            )
            .unwrap_err(),
        RunTransition::WrongState {
            status: RunStatus::Blocked,
            operation: "finish",
        }
    );
}

/// The full recoverable round trip: block, re-attach under another provider,
/// then succeed.
#[test]
fn a_blocked_run_resumed_cross_provider_can_still_succeed() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();
    receipt
        .resume(session("02"), Provider::OpenCode, "T2")
        .unwrap();

    receipt
        .terminate(
            RunOutcome::Succeeded,
            report(),
            diff(),
            session("02"),
            Provider::OpenCode,
            "T3",
        )
        .unwrap();

    assert_eq!(receipt.status, RunStatus::Succeeded);
    assert_eq!(receipt.id, run_id());
    let kinds: Vec<_> = receipt.checkpoints.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            CheckpointKind::Started,
            CheckpointKind::Blocked,
            CheckpointKind::Resumed,
            CheckpointKind::Terminated,
        ]
    );
}

// -- terminal states refuse everything --------------------------------------

/// Nothing may change a run that is over — and the refusal says "already
/// over", not "wrong state", because those are two different fixes.
#[test]
fn every_transition_on_a_terminal_run_is_refused_as_already_terminal() {
    let mut base = started();
    base.terminate(
        RunOutcome::Succeeded,
        report(),
        diff(),
        session("01"),
        Provider::ClaudeCode,
        "T1",
    )
    .unwrap();

    let mut receipt = base.clone();
    assert_eq!(
        receipt
            .resume(session("02"), Provider::ClaudeCode, "T2")
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "resume",
        }
    );

    let mut receipt = base.clone();
    assert_eq!(
        receipt
            .block(report(), diff(), session("02"), Provider::ClaudeCode, "T2")
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "block",
        }
    );

    let mut receipt = base.clone();
    assert_eq!(
        receipt
            .diverge("plan-fp-2", None, session("02"), Provider::ClaudeCode, "T2")
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "diverge",
        }
    );

    let mut receipt = base.clone();
    assert_eq!(
        receipt
            .accept_revision("plan-fp-2", session("02"), Provider::ClaudeCode, "T2")
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "accept-revision",
        }
    );

    let mut receipt = base.clone();
    assert_eq!(
        receipt
            .terminate(
                RunOutcome::Failed,
                report(),
                diff(),
                session("02"),
                Provider::ClaudeCode,
                "T2",
            )
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "finish",
        }
    );

    let mut receipt = base;
    assert_eq!(
        receipt.interrupt("T2").unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Succeeded,
            operation: "restart",
        }
    );
}

// -- interrupt --------------------------------------------------------------

/// The run reported no outcome, and inventing one would be exactly the
/// dishonesty this state exists to avoid.
#[test]
fn interrupting_terminalizes_without_inventing_an_outcome() {
    let mut receipt = started();

    receipt.interrupt("T1").unwrap();

    assert_eq!(receipt.status, RunStatus::Interrupted);
    assert_eq!(receipt.outcome, None);
    assert_eq!(receipt.terminated_at.as_deref(), Some("T1"));
    assert!(!receipt.holds_lock());
    assert_eq!(
        receipt.checkpoints.last().unwrap().kind,
        CheckpointKind::Interrupted
    );
}

#[test]
fn interrupting_keeps_everything_collected_so_far() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();

    receipt.interrupt("T2").unwrap();

    assert_eq!(receipt.status, RunStatus::Interrupted);
    assert_eq!(receipt.checkpoints.len(), 3);
    assert_eq!(receipt.checkpoints[1].report.as_ref(), Some(&report()));
}

/// Every non-terminal state can be abandoned — that is what `--restart` is
/// for, and a diverged run is exactly the one a human is most likely to give
/// up on.
#[test]
fn any_non_terminal_run_can_be_interrupted() {
    let mut blocked = started();
    blocked
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();
    assert!(blocked.interrupt("T2").is_ok());

    let mut diverged = started();
    diverged
        .diverge("plan-fp-2", None, session("01"), Provider::ClaudeCode, "T1")
        .unwrap();
    assert!(diverged.interrupt("T2").is_ok());

    assert!(started().interrupt("T1").is_ok());
}

/// The last coordinator is carried onto the interrupted checkpoint, so the
/// receipt still says who was holding the run when it was abandoned.
#[test]
fn an_interrupted_checkpoint_names_the_coordinator_that_held_the_run() {
    let mut receipt = started();
    receipt
        .resume(session("02"), Provider::OpenCode, "T1")
        .unwrap();

    receipt.interrupt("T2").unwrap();

    let checkpoint = receipt.checkpoints.last().unwrap();
    assert_eq!(checkpoint.session.as_ref(), Some(&session("02")));
    assert_eq!(checkpoint.provider, Some(Provider::OpenCode));
}

// -- legacy import ----------------------------------------------------------

#[test]
fn an_imported_board_becomes_a_terminal_legacy_receipt() {
    let receipt = RunReceipt::from_legacy(
        run_id(),
        feature(),
        "plans/checkout/plan.md",
        RunStatus::Interrupted,
        None,
        legacy_evidence(),
        "T0",
    );

    assert_eq!(receipt.provenance, RunProvenance::LegacyImport);
    assert_eq!(receipt.status, RunStatus::Interrupted);
    assert!(!receipt.holds_lock());
    assert_eq!(receipt.terminated_at.as_deref(), Some("T0"));
    assert!(receipt.coordinators.is_empty());
    assert_eq!(receipt.baseline, RunBaseline::empty());
    assert_eq!(receipt.checkpoints[0].kind, CheckpointKind::LegacyImport);
    assert_eq!(receipt.checkpoints[0].session, None);
    assert_eq!(receipt.legacy.as_ref(), Some(&legacy_evidence()));
}

/// A completed board keeps its outcome — the import is not allowed to
/// downgrade a run that really did succeed.
#[test]
fn an_imported_completed_board_keeps_its_outcome() {
    let receipt = RunReceipt::from_legacy(
        run_id(),
        feature(),
        "plans/checkout/plan.md",
        RunStatus::Succeeded,
        Some(RunOutcome::Succeeded),
        legacy_evidence(),
        "T0",
    );

    assert_eq!(receipt.status, RunStatus::Succeeded);
    assert_eq!(receipt.outcome, Some(RunOutcome::Succeeded));
}

/// A board with no graph fingerprint leaves nothing to pin to, and an empty
/// string is the honest answer — an imported receipt is terminal, so nothing
/// will ever compare it.
#[test]
fn an_imported_board_without_a_fingerprint_pins_to_nothing() {
    let mut evidence = legacy_evidence();
    evidence.plan_fingerprint = None;

    let receipt = RunReceipt::from_legacy(
        run_id(),
        feature(),
        "plans/checkout/plan.md",
        RunStatus::Interrupted,
        None,
        evidence,
        "T0",
    );

    assert_eq!(receipt.plan_fingerprint, "");
}

/// Nothing in the active lifecycle may resurrect an imported run: it is
/// terminal, and every transition says so.
#[test]
fn an_imported_receipt_cannot_be_resumed() {
    let mut receipt = RunReceipt::from_legacy(
        run_id(),
        feature(),
        "plans/checkout/plan.md",
        RunStatus::Interrupted,
        None,
        legacy_evidence(),
        "T0",
    );

    assert_eq!(
        receipt
            .resume(session("01"), Provider::ClaudeCode, "T1")
            .unwrap_err(),
        RunTransition::AlreadyTerminal {
            status: RunStatus::Interrupted,
            operation: "resume",
        }
    );
}

// -- serialization ----------------------------------------------------------

/// The whole aggregate round-trips, including the optional halves that only
/// serialize when they hold something.
#[test]
fn a_receipt_round_trips_through_json() {
    let mut receipt = started();
    receipt
        .block(report(), diff(), session("01"), Provider::ClaudeCode, "T1")
        .unwrap();
    receipt
        .resume(session("02"), Provider::OpenCode, "T2")
        .unwrap();
    receipt
        .terminate(
            RunOutcome::Succeeded,
            report(),
            diff(),
            session("02"),
            Provider::OpenCode,
            "T3",
        )
        .unwrap();

    let text = serde_json::to_string(&receipt).unwrap();
    let read_back: RunReceipt = serde_json::from_str(&text).unwrap();

    assert_eq!(read_back, receipt);
}

/// `deny_unknown_fields` on the receipt itself: a key nobody wrote is a
/// hand-edit or a newer binary, and either way reading it as if it were
/// understood is worse than refusing.
#[test]
fn a_receipt_carrying_an_unknown_field_does_not_deserialize() {
    let mut value = serde_json::to_value(started()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("native_session".to_owned(), serde_json::json!("abc"));

    assert!(serde_json::from_value::<RunReceipt>(value).is_err());
}

/// The persisted maps are ordered, so two reads of the same state render
/// identically — no diff noise from a hash map's iteration order.
#[test]
fn persisted_evidence_maps_are_ordered_by_key() {
    let mut repos = BTreeMap::new();
    for name in ["valhalla", "ivar", "orca"] {
        repos.insert(
            name.to_owned(),
            RepoDiff {
                head: "c0ffee".to_owned(),
                changes: BTreeMap::new(),
            },
        );
    }

    let text = serde_json::to_string(&RunDiff { repos }).unwrap();

    let ivar = text.find("ivar").unwrap();
    let orca = text.find("orca").unwrap();
    let valhalla = text.find("valhalla").unwrap();
    assert!(
        ivar < orca && orca < valhalla,
        "keys must serialize in order: {text}"
    );
}

/// Forward-compatible where it has to be: a receipt written before a list
/// existed still reads, because those fields carry `#[serde(default)]`.
#[test]
fn a_receipt_missing_its_defaulted_lists_still_reads() {
    let json = r#"{
        "version": 1,
        "id": "6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33",
        "feature": "checkout",
        "provenance": "native",
        "status": "active",
        "plan_path": "plans/checkout/plan.md",
        "plan_fingerprint": "plan-fp-1",
        "started_at": "T0",
        "updated_at": "T0"
    }"#;

    let receipt: RunReceipt = serde_json::from_str(json).unwrap();

    assert!(receipt.coordinators.is_empty());
    assert!(receipt.checkpoints.is_empty());
    assert_eq!(receipt.baseline, RunBaseline::empty());
}

// -- failures ---------------------------------------------------------------

/// Each refusal carries the one command that resolves it — a diverged run
/// points at accept-revision, a blocked one at resume.
#[test]
fn a_wrong_state_refusal_names_the_command_that_unsticks_it() {
    let diverged = Failure::from(RunTransition::WrongState {
        status: RunStatus::Diverged,
        operation: "resume",
    });
    assert_eq!(diverged.code, "execute.run_wrong_state");
    assert!(
        diverged.fix_actions[0].what.contains("accept-revision"),
        "a diverged run must be pointed at accept-revision"
    );

    let blocked = Failure::from(RunTransition::WrongState {
        status: RunStatus::Blocked,
        operation: "finish",
    });
    assert!(blocked.fix_actions[0].what.contains("--resume"));
}

#[test]
fn a_terminal_refusal_points_at_starting_a_new_run() {
    let failure = Failure::from(RunTransition::AlreadyTerminal {
        status: RunStatus::Succeeded,
        operation: "resume",
    });

    assert_eq!(failure.code, "execute.run_terminal");
    assert!(failure.fix_actions[0].what.contains("start"));
}

#[test]
fn an_unknown_outcome_lists_the_ones_that_exist() {
    let failure = Failure::from(UnknownRunOutcome("done".to_owned()));

    assert_eq!(failure.code, "execute.unknown_outcome");
    assert!(failure.what.contains("succeeded"));
}
