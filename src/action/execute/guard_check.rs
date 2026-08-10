//! `ivar feature execute guard-check --session <id> --path <path>` — check
//! whether a path is allowed by the write contract of the workstream that
//! owns the given provider session.
//!
//! # What it does
//!
//! Reads the execution board for `input.feature`, looks up `session` in the
//! `sessions` map to find the owning workstream, and checks whether
//! `path` is covered by that workstream's [`WriteContract`](crate::domain::feature::WriteContract).
//!
//! The default is DENY: unknown session, missing board, unreadable board — all
//! refuse. A path inside the contract passes; outside is refused naming the
//! workstream.

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::{ExecutionBoard, WriteContract};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use super::find_workstream;

/// What `ivar feature execute guard-check` needs.
#[derive(Debug, Clone)]
pub struct GuardCheckInput {
    /// The feature whose board holds the session→workstream link.
    pub feature: Option<String>,
    /// Provider session id to look up in the board's `sessions` map.
    pub session: Option<String>,
    /// Path to check against the workstream's write contract.
    pub path: Option<String>,
}

/// What `ivar feature execute guard-check` did.
#[derive(Debug, Clone, Serialize)]
pub struct GuardCheckOutcome {
    /// Whether the path is allowed.
    pub allowed: bool,
    /// The workstream that owns the session, when known.
    pub workstream: Option<String>,
    /// Where the board was read from.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for GuardCheckOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        let status = if self.allowed { "allowed" } else { "denied" };
        writeln!(
            w,
            "guard-check: {} ({})",
            status,
            self.workstream.as_deref().unwrap_or("unknown session")
        )
    }
}

/// Check whether `input.path` is allowed by the write contract of the
/// workstream that owns `input.session`.
///
/// Blocked when the board is missing, any required argument is absent, or the
/// path falls outside the workstream's write contract. The default is DENY.
pub fn guard_check(ctx: &Ctx, input: GuardCheckInput) -> Outcome<GuardCheckOutcome> {
    // Validate arguments before touching the hall — a missing argument is a
    // caller error, not a hall problem.
    let feature_name = require_feature(&input)?;
    let path = require_path(&input)?;
    let session = require_session(&input)?;

    let layout = discover_hall(ctx)?;

    let board = match ExecutionBoard::read(&layout, &feature_name) {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: None,
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
        Err(e) => {
            return Err(e);
        }
    };

    // Look up the session → workstream link.
    let workstream_id = match board.sessions.get(session) {
        Some(id) => id.clone(),
        None => {
            // Unknown session — never allowed by omission.
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: None,
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
    };

    // Find the workstream definition to get its write contract.
    let workstream = match find_workstream(&board, &feature_name, &workstream_id) {
        Ok(ws) => ws,
        Err(_) => {
            // Session references a workstream not found on the board — deny.
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: Some(workstream_id),
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
    };

    let contract = WriteContract::new(workstream.write_contract.clone());
    let resolved = ctx.resolve(&path);

    let allowed = contract.allows(&resolved);

    Ok(Report::new(GuardCheckOutcome {
        allowed,
        workstream: Some(workstream_id),
        board_path: crate::store::feature::board_path(&layout, &feature_name),
    }))
}

fn require_feature(input: &GuardCheckInput) -> Result<FeatureName, Failure> {
    let feature = input.feature.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_feature",
            "--feature is required".to_owned(),
        )
    })?;
    Ok(FeatureName::new(feature)?)
}

fn require_session(input: &GuardCheckInput) -> Result<&str, Failure> {
    input.session.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_session",
            "--session is required".to_owned(),
        )
    })
}

fn require_path(input: &GuardCheckInput) -> Result<Utf8PathBuf, Failure> {
    input.path.as_deref().map(Utf8PathBuf::from).ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_path",
            "--path is required".to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::execute::prepare::{
        self as prepare_action, PrepareInput as PrepareActionInput,
    };
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-src",
                "title": "Source files",
                "operations": ["write-code"],
                "depends_on": [],
                "write_contract": ["src/"]
            },
            {
                "id": "ws-docs",
                "title": "Docs",
                "operations": ["write-docs"],
                "depends_on": [],
                "write_contract": ["docs/"]
            }
        ]
    }"#;

    /// A hall with a prepared board, sessions injected manually.
    fn seeded_board() -> (tempfile::TempDir, Utf8PathBuf) {
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

        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        prepare_action::prepare(
            &ctx,
            PrepareActionInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();

        // Inject sessions into the board.
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        board
            .sessions
            .insert("sess-src".to_owned(), "ws-src".to_owned());
        board
            .sessions
            .insert("sess-docs".to_owned(), "ws-docs".to_owned());
        board.write(&layout, &feature).unwrap();

        (guard, root)
    }

    #[test]
    fn path_inside_the_workstream_contract_is_allowed() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: Some("src/main.rs".to_owned()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(outcome.value.allowed);
        assert_eq!(outcome.value.workstream.as_deref(), Some("ws-src"));
    }

    #[test]
    fn path_outside_the_workstream_contract_is_denied_naming_the_workstream() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: Some("docs/guide.md".to_owned()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(!outcome.value.allowed);
        assert_eq!(outcome.value.workstream.as_deref(), Some("ws-src"));
    }

    #[test]
    fn unknown_session_is_never_allowed() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-ghost".to_owned()),
                path: Some("src/main.rs".to_owned()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(!outcome.value.allowed);
        assert!(outcome.value.workstream.is_none());
    }

    #[test]
    fn dot_dot_does_not_escape_the_contract() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        // ".." in path should be rejected by WriteContract::allows.
        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: Some("../outside.txt".to_owned()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(!outcome.value.allowed);
    }

    #[test]
    fn missing_feature_argument_returns_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());

        let failure = guard_check(
            &ctx,
            GuardCheckInput {
                feature: None,
                session: Some("sess-src".to_owned()),
                path: Some("src/main.rs".to_owned()),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, crate::error::Status::Blocked);
        assert_eq!(failure.code, "execute.guard_check.missing_feature");
    }

    #[test]
    fn missing_session_argument_returns_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());

        let failure = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: None,
                path: Some("src/main.rs".to_owned()),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, crate::error::Status::Blocked);
        assert_eq!(failure.code, "execute.guard_check.missing_session");
    }

    #[test]
    fn missing_path_argument_returns_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());

        let failure = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, crate::error::Status::Blocked);
        assert_eq!(failure.code, "execute.guard_check.missing_path");
    }

    #[test]
    fn absent_board_returns_denied_with_no_workstream() {
        // A hall that exists but whose board was never prepared: the guard
        // must deny by default — an unreadable board is a denial, never a
        // grant.
        let (_guard, root) = hall_root();
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

        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: Some("src/main.rs".to_owned()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(!outcome.value.allowed);
        assert!(outcome.value.workstream.is_none());
    }
}
