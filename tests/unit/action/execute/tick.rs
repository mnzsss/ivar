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

use super::*;
use crate::action::execute::approve::{self as approve_action, ApproveInput};
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
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
    assert_eq!(persisted(&root).status, ExecutionStatus::Running);

    // Session→workstream link recorded.
    let on_disk = persisted(&root);
    assert_eq!(on_disk.sessions.len(), 1);
    let (sess_id, ws_id) = on_disk.sessions.iter().next().unwrap();
    assert_eq!(ws_id, "ws-b");
    assert!(!sess_id.is_empty());
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
