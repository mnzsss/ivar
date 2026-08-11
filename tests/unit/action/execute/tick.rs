//! Unit tests for `crate::action::execute::tick`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use super::*;
use crate::action::execute::approve::{self as approve_action, ApproveInput};
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::execute::reply::{self as reply_action, ReplyInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::WriteContract;
use crate::domain::name::RepoName;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-a",
            "title": "A",
            "operations": ["op-a"],
            "depends_on": [],
            "write_contract": ["src/a"]
        },
        {
            "id": "ws-b",
            "title": "B",
            "operations": ["op-b"],
            "depends_on": ["ws-a"],
            "write_contract": ["src/b"]
        }
    ]
}"#;

/// A plan whose `## Operations` section backs every operation
/// `GRAPH_JSON` claims — `tick` now renders each workstream's prompt from
/// this text via [`prompt::render`], which refuses a workstream that
/// claims an operation the plan does not document.
const PLAN_TEXT: &str = "# Plan\n\
    \n\
    ## Operation details\n\
    \n\
    **op-a** — Do the first thing.\n\
    \n\
    **op-b** — Do the second thing.\n\
    \n\
    ## Operations\n\
    \n\
    ### ws-a\n\
    - op-a\n\
    write_contract:\n\
    - src/a\n\
    \n\
    ### ws-b\n\
    - op-b\n\
    write_contract:\n\
    - src/b\n";

/// A hall with a feature, a plan, and a prepared+approved board.
fn approved_board() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    // Overwrite the scaffolded plan.md with one that documents op-a/op-b
    // under each workstream's own heading, before `prepare` fingerprints
    // it — the fingerprint prepare records has to match what is on disk
    // by the time `tick` reads it back, or the divergence check fires.
    fs::write_text(&root.join("plans/checkout/plan.md"), PLAN_TEXT).unwrap();
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    approve_action::approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    (guard, root)
}

/// The board read back off disk.
fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
}

#[test]
fn tick_launches_workstreams_with_met_dependencies() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Mark ws-a as Done so ws-b becomes ready.
    {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        for ws in &mut board.graph.workstreams {
            if ws.id == "ws-a" {
                ws.status = WorkstreamStatus::Done;
            }
        }
        board.write(&layout, &feature).unwrap();
    }

    let _stub = PathStub::install("claude", "exit 0");
    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.launched, vec!["ws-b"]);
    assert_eq!(persisted(&root).status, ExecutionStatus::Completed);

    // Session→workstream link recorded.
    let on_disk = persisted(&root);
    assert_eq!(on_disk.sessions.len(), 1);
    let (sess_id, ws_id) = on_disk.sessions.iter().next().unwrap();
    assert_eq!(ws_id, "ws-b");
    assert!(!sess_id.is_empty());
}

#[test]
fn tick_warns_when_executor_cannot_materialise_canonical_instructions() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());
    fs::remove_file(&root.join("HALL.md")).unwrap();

    let _stub = PathStub::install("claude", "exit 0");
    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.warnings.len(), 1);
    assert_eq!(
        report.warnings[0].code,
        "instructions.canonical_unavailable"
    );
}

#[test]
fn tick_blocks_when_plan_diverges() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Tamper with plan.md behind ivar's back — changes the fingerprint.
    fs::write_text(
        &root.join("plans/checkout/plan.md"),
        "# Plan\n\n- changed content\n",
    )
    .unwrap();

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(
        report.value.launched.is_empty(),
        "divergent plan must block"
    );

    // Workstream marked as Blocked.
    let on_disk = persisted(&root);
    assert_eq!(
        on_disk.graph.workstreams[0].status,
        WorkstreamStatus::Blocked
    );

    // Journal records the divergence.
    let last_entry = on_disk.journal.last().unwrap();
    assert_eq!(last_entry.kind, "diverged");
}

#[test]
fn tick_with_nothing_ready_is_a_no_op() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // No workstream is Done, so ws-b's dependency is unmet.
    // ws-a has no dependencies but... actually ws-a HAS no deps and IS
    // Waiting, so it should launch. Let me check: depends_on is empty
    // for ws-a, so deps_met is true, meaning ws-a WILL launch.
    // To test "nothing ready", I need to set up a scenario where no
    // workstream can launch. Let me modify the setup.

    // Actually, with the current graph, ws-a has no deps and will always
    // launch. Let me create a board where ws-a depends on itself (circular)
    // or just accept that ws-a launches and test differently.

    // For "nothing ready" test, let me manually set up a board where
    // ws-a depends on ws-b which depends on ws-a (circular deps).
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

    // Make both depend on each other — circular, neither can launch.
    board.graph.workstreams[0].depends_on = vec!["ws-b".to_owned()];
    board.graph.workstreams[1].depends_on = vec!["ws-a".to_owned()];

    board.write(&layout, &feature).unwrap();

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(
        report.value.launched.is_empty(),
        "circular deps: nothing should launch"
    );
}

#[test]
fn tick_refuses_a_board_not_in_approved_status() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Set board to Pending.
    {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        board.set_status(ExecutionStatus::Pending);
        board.write(&layout, &feature).unwrap();
    }

    let failure = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.board_not_approved");
    assert!(failure.what.contains("pending"));
}

#[test]
fn the_agent_reaches_the_provider_command_line() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Give ws-a a custom agent.
    {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        if let Some(ws) = board
            .graph
            .workstreams
            .iter_mut()
            .find(|ws| ws.id == "ws-a")
        {
            ws.agent = Some("custom-agent".to_owned());
        }
        board.write(&layout, &feature).unwrap();
    }

    let _stub = PathStub::install("claude", "exit 0");
    let _report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    // Verify via journal entry that the agent appears in the command.
    let on_disk = persisted(&root);
    let started_entries: Vec<_> = on_disk
        .journal
        .iter()
        .filter(|e| e.kind == "started")
        .collect();
    assert!(!started_entries.is_empty(), "must have started entries");
    let msg = &started_entries[0].message;
    assert!(msg.contains("custom-agent"), "agent must reach CLI: {msg}");
}

#[test]
fn the_session_to_workstream_link_is_recorded() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 0");
    let _report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let on_disk = persisted(&root);
    assert_eq!(
        on_disk.sessions.len(),
        1,
        "exactly one session launched (ws-a)"
    );
    let (session_id, workstream_id) = on_disk.sessions.iter().next().unwrap();
    assert_eq!(workstream_id, "ws-a");
    // Session ID is a valid UUID.
    assert!(uuid::Uuid::parse_str(session_id).is_ok());
}

// --- the child's git identity ------------------------------------------

/// The child's environment carries the ivar session variables and
/// nothing this module adds beyond them — never a git identity override.
/// The stub dumps its own `env` to a file (the only reliable way to
/// inspect a child's actual environment) rather than the test reading
/// `/proc` or similar.
#[test]
fn the_child_env_carries_ivar_session_vars_but_never_a_git_identity() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let (_env_guard, env_dir) = crate::test_support::utf8_temp_dir();
    let env_dump = env_dir.join("child-env.txt");
    let _stub = PathStub::install("claude", &format!("env > '{env_dump}'\n"));

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert!(report.is_clean());

    let dumped = fs::read_text(&env_dump).unwrap().unwrap();
    assert!(
        dumped.contains("IVAR_SESSION_ID="),
        "ivar session vars must reach the child: {dumped}"
    );
    assert!(dumped.contains("IVAR_FEATURE=checkout"), "was: {dumped}");
    for var in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
    ] {
        assert!(
            !dumped
                .lines()
                .any(|line| line.starts_with(&format!("{var}="))),
            "{var} must never reach the child: {dumped}"
        );
    }
}

// --- folding the provider's events into the board -----------------------

/// A tool call and the provider's native session id, both emitted on the
/// stub's stdout as real `stream-json` lines, fold into the journal as
/// `tool.used` and `native_session` — and a clean exit reaches `Done`.
#[test]
fn stream_events_fold_into_the_journal_and_a_clean_exit_reaches_done() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let script = r#"printf '%s\n' '{"type":"system","subtype":"init","session_id":"native-xyz-789"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/a/lib.rs"}}]}}'
exit 0
"#;
    let _stub = PathStub::install("claude", script);

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert!(report.is_clean());

    let on_disk = persisted(&root);
    assert!(
        on_disk.journal.iter().any(|e| e.kind == "tool.used"
            && e.message.contains("Read")
            && e.message.contains("src/a/lib.rs")),
        "tool.used must be in the journal: {:?}",
        on_disk.journal
    );
    assert!(
        on_disk
            .journal
            .iter()
            .any(|e| e.kind == "native_session" && e.message.contains("native-xyz-789")),
        "the native session id must be persisted: {:?}",
        on_disk.journal
    );
    assert!(
        on_disk
            .journal
            .iter()
            .any(|e| e.kind == "session.completed"),
        "a clean exit must journal session.completed: {:?}",
        on_disk.journal
    );

    let ws_a = on_disk
        .graph
        .workstreams
        .iter()
        .find(|w| w.id == "ws-a")
        .unwrap();
    assert_eq!(ws_a.status, WorkstreamStatus::Done);
}

// --- terminal status ----------------------------------------------------

/// A process that exits non-zero reaches a terminal status — never left
/// `Active` — and a later `tick` gets a clear, structured refusal rather
/// than hanging or silently doing nothing: the board is recoverable via
/// `reply`, not by deleting and re-preparing it.
#[test]
fn a_failed_child_reaches_a_terminal_status_not_active_forever() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "printf trouble >&2\nexit 3\n");

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert!(report.is_clean());

    let on_disk = persisted(&root);
    let ws_a = on_disk
        .graph
        .workstreams
        .iter()
        .find(|w| w.id == "ws-a")
        .unwrap();
    assert_ne!(
        ws_a.status,
        WorkstreamStatus::Active,
        "a dead process must never leave its workstream Active"
    );
    assert_eq!(ws_a.status, WorkstreamStatus::Blocked);
    assert_eq!(on_disk.status, ExecutionStatus::Blocked);
    assert_eq!(on_disk.blocked_by.as_deref(), Some("ws-a"));
    assert!(
        on_disk.journal.iter().any(|e| e.kind == "session.failed"
            && e.message.contains("exited 3")
            && e.message.contains("trouble")),
        "the failure must be journaled with its exit code and stderr: {:?}",
        on_disk.journal
    );

    // A later tick does not hang and does not need the board deleted: it
    // gets a clear, structured refusal naming the blocked status.
    let second = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(second.code, "execute.board_not_approved");
}

// -- OP-TEST-REPRO: prove the bug before fixing it -----------------------
//
// A stub `claude` goes on `PATH` — see the module-level comment on
// `TEST_STUB_BIN_DIR` for why a thread-local rather than mutating the
// process's real `PATH`. While the guard is alive, anything this test's
// thread builds a spawn command on sees the stub first.

/// RAII: while alive, an executable named `name` running `script_body`
/// (under `#!/bin/sh`) is first on the `PATH` this test's thread builds
/// spawn commands against. Dropping clears the seam.
struct PathStub {
    _dir: tempfile::TempDir,
}

impl PathStub {
    fn install(name: &str, script_body: &str) -> Self {
        let (dir, bin_dir) = crate::test_support::utf8_temp_dir();
        let script_path = bin_dir.join(name);
        fs::write_text(&script_path, &format!("#!/bin/sh\n{script_body}\n")).unwrap();
        fs::chmod(&script_path, 0o755).unwrap();
        TEST_STUB_BIN_DIR.with(|cell| *cell.borrow_mut() = Some(bin_dir));
        Self { _dir: dir }
    }
}

impl Drop for PathStub {
    fn drop(&mut self) {
        TEST_STUB_BIN_DIR.with(|cell| *cell.borrow_mut() = None);
    }
}

/// The bug, proven: a stub `claude` on `PATH` touches a sentinel file the
/// instant it runs. `tick`'s current loop builds a command *string* and
/// never spawns anything, so the sentinel must never appear — this is an
/// external effect, not a journal or `board.sessions` assertion, because
/// the current fake harness already writes those correctly (that is
/// exactly how it hides the bug). Demonstrated failing against the
/// unmodified launch loop, before the real spawn replaced it.
#[test]
fn tick_actually_spawns_the_provider() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let (_sentinel_guard, sentinel_dir) = crate::test_support::utf8_temp_dir();
    let sentinel = sentinel_dir.join("claude-ran");
    let _stub = PathStub::install("claude", &format!("touch '{sentinel}'\n"));

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(
        fs::is_file(&sentinel).unwrap(),
        "tick must actually spawn `claude`, not just record a fake session"
    );
}

// -- wave handoff --------------------------------------------------------

/// A tick launches one wave and blocks until it finishes. The board it
/// leaves behind has to be tickable again, or every wave after the first is
/// stranded: `tick` demands `approved`, and a board parked at `running`
/// refuses every command that could move it — `tick` (not approved),
/// `approve` (not awaiting approval) and `reply` (not blocked) alike.
#[test]
fn a_finished_wave_returns_the_board_to_approved_so_the_next_can_launch() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 0");

    let first = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(first.value.launched, vec!["ws-a"]);

    let after_first = persisted(&root);
    assert_eq!(
        after_first.status,
        ExecutionStatus::Approved,
        "a wave that finished with work still waiting must leave the board tickable"
    );
    assert!(
        after_first
            .journal
            .iter()
            .any(|entry| entry.kind == "wave.completed"),
        "the handoff back to `approved` must be journalled: {:?}",
        after_first.journal
    );

    // The next wave launches from that same board — no re-approval, no
    // re-prepare.
    let second = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(second.value.launched, vec!["ws-b"]);

    let after_second = persisted(&root);
    assert_eq!(
        after_second.status,
        ExecutionStatus::Completed,
        "a board whose every workstream is done is completed, not running"
    );
    assert!(
        after_second
            .journal
            .iter()
            .any(|entry| entry.kind == "board.completed"),
        "the completion must be journalled: {:?}",
        after_second.journal
    );
}

/// A blocked wave stays blocked: settling after the fold loop must not
/// launder a board that needs a human back into `approved`.
#[test]
fn a_blocked_wave_stays_blocked_after_the_tick_settles() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 7");

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let on_disk = persisted(&root);
    assert_eq!(on_disk.status, ExecutionStatus::Blocked);
    assert_eq!(on_disk.blocked_by.as_deref(), Some("ws-a"));
}

/// The whole recovery loop, end to end: a workstream blocks, a human
/// replies, and the next tick relaunches it — with the answer in its prompt.
/// A relaunch that dropped the answer would hand the agent the exact prompt
/// that produced the question and get the same question back, forever.
#[test]
fn a_replied_workstream_relaunches_with_the_answer_in_its_prompt() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Wave 1: the child dies, so the board blocks on `ws-a`.
    {
        let _stub = PathStub::install("claude", "exit 3");
        tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
    }
    let blocked = persisted(&root);
    assert_eq!(blocked.status, ExecutionStatus::Blocked);
    let session = blocked
        .sessions
        .iter()
        .find(|(_, workstream)| workstream.as_str() == "ws-a")
        .map(|(id, _)| id.clone())
        .unwrap();

    reply_action::reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some(session),
            message: "use the v2 endpoint".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        persisted(&root).status,
        ExecutionStatus::Approved,
        "a reply must leave the board tickable"
    );

    // Wave 2: capture the prompt the relaunched child is handed. `claude`
    // takes it as the argument to `-p`, so `$2` is the prompt itself.
    let (_sentinel_guard, sentinel_dir) = crate::test_support::utf8_temp_dir();
    let sentinel = sentinel_dir.join("prompt.txt");
    let _stub = PathStub::install("claude", &format!("printf '%s' \"$2\" > '{sentinel}'"));

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(report.value.launched, vec!["ws-a"]);

    let prompt_text = fs::read_text(&sentinel).unwrap().unwrap();
    assert!(
        prompt_text.contains("use the v2 endpoint"),
        "the relaunched prompt must carry the human's answer: {prompt_text}"
    );
}

// -- the post-run write-contract audit ----------------------------------

/// The audit's whole reason for existing: the execution guard is a
/// `PreToolUse` hook on `Write|Edit|MultiEdit|NotebookEdit`, and `Bash`
/// carries a command rather than a path, so a shell write reaches the disk
/// without the guard ever being asked. This is what sees it afterwards.
#[test]
fn a_path_outside_every_contract_in_the_wave_is_a_violation() {
    // A contract names directories with a trailing `/` and files by name —
    // the shapes `WriteContract::allows` arbitrates. The paths are
    // `<repo>/<path>`, which is what `launch::audit_path` builds and what a
    // contract is written against.
    let contract = WriteContract::new(vec!["src/a/".to_owned(), "src/b/".to_owned()]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [
        Utf8PathBuf::from("api-main/src/a/one.rs"),
        Utf8PathBuf::from("api-main/src/b/two.rs"),
        Utf8PathBuf::from("api-main/src/elsewhere/three.rs"),
    ]
    .into_iter()
    .collect();

    let violations = launch::contract_violations(&contract, &baseline, &after);

    assert_eq!(
        violations,
        vec![Utf8PathBuf::from("api-main/src/elsewhere/three.rs")]
    );
}

/// A wave shares its worktrees, so a sibling's legitimate write shows up in
/// this workstream's `git status` exactly like its own stray one. Measuring
/// against the wave's union is what keeps the audit from blaming `ws-a` for
/// the file `ws-b` was launched to write.
#[test]
fn a_sibling_workstream_s_own_files_are_not_reported_as_violations() {
    // The union `tick` builds: `ws-a` owns `src/a`, `ws-b` owns `src/b`.
    let wave = WriteContract::new(vec!["src/a/".to_owned(), "src/b/".to_owned()]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [Utf8PathBuf::from("api-main/src/b/two.rs")]
        .into_iter()
        .collect();

    assert!(launch::contract_violations(&wave, &baseline, &after).is_empty());
}

/// What was already dirty when the workstream started is not what it wrote —
/// an uncommitted human edit, or an earlier tick's work, must not fail the
/// next workstream that runs beside it.
#[test]
fn what_was_already_dirty_before_the_run_is_not_a_violation() {
    let contract = WriteContract::new(vec!["src/a/".to_owned()]);
    let inherited = Utf8PathBuf::from("api-main/notes.md");
    let baseline: BTreeSet<Utf8PathBuf> = [inherited.clone()].into_iter().collect();
    let after: BTreeSet<Utf8PathBuf> = [inherited].into_iter().collect();

    assert!(launch::contract_violations(&contract, &baseline, &after).is_empty());
}

/// An empty contract allows nothing — the domain's own default-deny — so a
/// workstream with no contract that writes anything at all is caught.
#[test]
fn an_empty_contract_allows_nothing() {
    let contract = WriteContract::new(Vec::new());
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [Utf8PathBuf::from("api-main/src/a/one.rs")]
        .into_iter()
        .collect();

    assert_eq!(
        launch::contract_violations(&contract, &baseline, &after).len(),
        1
    );
}

/// A real write contract names its files `<repo>/<path>`, the shape of a
/// session view dir. Matching that against a path built from a worktree root
/// wedges the branch segment in the middle, `ends_with` never fires, and every
/// new write in every repo is reported as a violation.
///
/// The `src/a/` contracts the tests above use match at any depth, so none of
/// them can see this.
#[test]
fn a_path_the_contract_names_by_repo_is_matched_against_the_repo_s_own_shape() {
    let repo = RepoName::new("gaio-backend").unwrap();
    let relative = Utf8PathBuf::from("packages/console/src/workflows/repositories/workflow.ts");

    let contract = WriteContract::new(vec![
        "gaio-backend/packages/console/src/workflows/repositories/workflow.ts".to_owned(),
    ]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [launch::audit_path(&repo, &relative)].into_iter().collect();

    assert!(
        launch::contract_violations(&contract, &baseline, &after).is_empty(),
        "the workstream's own contracted file must not be reported as a violation"
    );
}
