//! `ivar session relay` — relay to a new session on the same feature under a
//! different provider.
//!
//! A thin alias over `session start --relay`: delegates to the same code path,
//! then formats the outcome as four lines of human-readable output:
//!
//! ```text
//! Session `<id>` for feature `<name>` relayed.
//! Provider: <provider>
//! plan preserved · N of M steps done
//!
//! ```
//!
//! The third line reads the execution board's workstream status: `N` is the
//! count of completed workstreams, `M` is the total. When no board exists,
//! it shows `0 of 0`.

use std::io;

use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::ExecutionBoard;
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::start;

/// What `ivar session relay` needs.
#[derive(Debug, Clone)]
pub struct RelayInput {
    /// The feature to relay on. Required — relay creates a new session on the
    /// same feature under a different provider.
    pub feature: String,
    /// The provider to relay to. Required — relay must switch providers.
    pub provider: String,
}

/// Output of `ivar session relay`: four lines of human-readable text.
#[derive(Debug, Clone, Serialize)]
pub struct RelayOutcome {
    /// The new session's id.
    pub session_id: String,
    /// The feature this session is bound to.
    pub feature: FeatureName,
    /// The provider that ran the relayed session.
    pub provider: crate::domain::provider::Provider,
    /// Steps done / total from the execution board (only when a board exists).
    pub steps_done: Option<u64>,
    pub steps_total: Option<u64>,
}

impl WriteHuman for RelayOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Session `{}` for feature `{}` relayed.",
            self.session_id, self.feature
        )?;
        writeln!(w, "Provider: {}", self.provider)?;
        match (self.steps_done, self.steps_total) {
            (Some(done), Some(total)) => {
                writeln!(w, "plan preserved · {done} of {total} steps done")?;
            }
            _ => {
                writeln!(w, "plan preserved · 0 of 0 steps done")?;
            }
        }
        // Fourth line: blank separator.
        writeln!(w)
    }
}

/// Relay: create a fresh session on the same feature under a different
/// provider. This is a thin wrapper around `start` with `relay=true` — if the
/// two paths diverge, that is the bug this operation exists to prevent.
pub fn relay(ctx: &Ctx, input: RelayInput) -> Outcome<RelayOutcome> {
    let layout = discover_hall(ctx)?;

    let feature_name = FeatureName::new(input.feature)?;

    // Delegate to start with relay flag — same gates, same logic.
    let report = start::start(
        ctx,
        start::StartInput {
            feature: feature_name.to_string(),
            resume: false,
            provider: Some(input.provider.clone()),
            detached: true,
            relay: true,
        },
    )?;

    let start_outcome = &report.value;

    // Read the execution board for the step count.
    let (steps_done, steps_total) = read_board_steps(&layout, &start_outcome.feature);

    Ok(Report::with_warnings(
        RelayOutcome {
            session_id: start_outcome.session_id.clone(),
            feature: start_outcome.feature.clone(),
            provider: start_outcome.provider,
            steps_done,
            steps_total,
        },
        report.warnings.clone(),
    ))
}

/// Read the execution board and return (done, total) workstream counts.
fn read_board_steps(layout: &Layout, feature_name: &FeatureName) -> (Option<u64>, Option<u64>) {
    match ExecutionBoard::read(layout, feature_name) {
        Ok(Some(board)) => {
            let total = board.graph.workstreams.len() as u64;
            let done = board
                .graph
                .workstreams
                .iter()
                .filter(|ws| ws.status == crate::domain::feature::WorkstreamStatus::Done)
                .count() as u64;
            (Some(done), Some(total))
        }
        Ok(None) | Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use camino::Utf8PathBuf;

    use super::*;
    use crate::action::feature::create::{self as feature_create, CreateInput};
    use crate::action::feature::promote::{self as feature_promote, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::action::session::start::{self as session_start, StartInput};
    use crate::domain::feature::{ExecutionGraph, WorkstreamDef, WorkstreamStatus};
    use crate::domain::name::{BranchName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    fn hall_with_provider_session() -> (tempfile::TempDir, Utf8PathBuf) {
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

        let origins = root.parent().unwrap().join("origins");
        let api_origin = seeded_repo(&origins.join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(
                vec![Provider::ClaudeCode, Provider::OpenCode],
                Provider::ClaudeCode,
            ),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        feature_create::create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        feature_promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        // A session on the default provider (claude-code).
        session_start::start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        (guard, root)
    }

    fn unguard_worktrees(root: &camino::Utf8Path) {
        let repos = root.join(".ivar/repos");
        if !fs::is_dir(&repos).unwrap() {
            return;
        }
        for repo in fs::read_dir(&repos).unwrap() {
            for worktree in fs::read_dir(&repo).unwrap() {
                let _ = fs::restore_write_bits(&worktree);
            }
        }
    }

    /// `session relay` and `session start --relay` produce the same outcome on
    /// the same state: same feature, same provider, same session record, same
    /// worktree reuse — differing only in the fresh session id. This is the
    /// test that prevents the two paths from diverging (the whole point of the
    /// verb being a thin alias).
    #[test]
    fn relay_and_start_relay_produce_the_same_outcome() {
        // Two identical halls: relay via the verb in one, via `start --relay`
        // in the other.
        let (_guard_a, root_a) = hall_with_provider_session();
        let ctx_a = Ctx::new(root_a.clone());
        let via_relay = relay(
            &ctx_a,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "opencode".to_owned(),
            },
        )
        .unwrap();

        let (_guard_b, root_b) = hall_with_provider_session();
        let ctx_b = Ctx::new(root_b.clone());
        let via_start = session_start::start(
            &ctx_b,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: Some("opencode".to_owned()),
                detached: true,
                relay: true,
            },
        )
        .unwrap();

        let a = &via_relay.value;
        let b = &via_start.value;
        assert_eq!(a.feature, b.feature, "same feature");
        assert_eq!(a.provider, b.provider, "same provider");
        assert_eq!(
            via_relay.is_clean(),
            via_start.is_clean(),
            "same warning set"
        );

        // Both created a real, fresh session bound to opencode on checkout.
        let state_a = session_state_of(&root_a, &a.session_id);
        let state_b = session_state_of(&root_b, &b.session_id);
        assert_eq!(state_a.provider(), Provider::OpenCode);
        assert_eq!(state_b.provider(), Provider::OpenCode);
        assert_eq!(state_a.feature().unwrap().as_str(), "checkout");
        assert_eq!(state_b.feature().unwrap().as_str(), "checkout");

        // Both reuse the feature worktree the previous session linked.
        assert!(
            api_link_target(&root_a, &a.session_id).contains(".ivar/repos/api/checkout"),
            "relay must reuse the feature worktree"
        );
        assert!(
            api_link_target(&root_b, &b.session_id).contains(".ivar/repos/api/checkout"),
            "start --relay must reuse the feature worktree"
        );

        unguard_worktrees(&root_a);
        unguard_worktrees(&root_b);
    }

    /// The four-line output is contract with the landing's first fold: session
    /// line, provider line, board line, blank separator.
    #[test]
    fn relay_emits_the_four_line_output_contract() {
        let (_guard, root) = hall_with_provider_session();
        let ctx = Ctx::new(root.clone());

        let relay_report = relay(
            &ctx,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "opencode".to_owned(),
            },
        )
        .unwrap();

        assert!(relay_report.is_clean());
        assert_eq!(relay_report.value.feature.as_str(), "checkout");
        assert_eq!(relay_report.value.provider, Provider::OpenCode);

        // Verify exactly 4 lines of output.
        let mut out = Vec::new();
        relay_report.value.write_human(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            4,
            "must produce exactly 4 lines, got {}: {:?}",
            lines.len(),
            lines
        );
        assert!(lines[0].contains("relayed"));
        assert!(lines[1].starts_with("Provider:"));
        assert!(lines[2].starts_with("plan preserved"));
        assert!(lines[3].is_empty(), "fourth line must be blank");
        unguard_worktrees(&root);
    }

    /// The third line counts the execution board's workstream steps.
    #[test]
    fn relay_third_line_counts_the_boards_steps() {
        let (_guard, root) = hall_with_provider_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();

        // A board with three workstreams, one done.
        let graph = ExecutionGraph {
            workstreams: vec![
                workstream("WS-1", WorkstreamStatus::Done),
                workstream("WS-2", WorkstreamStatus::Active),
                workstream("WS-3", WorkstreamStatus::Waiting),
            ],
            plan_fingerprint: "abc123".to_owned(),
        };
        ExecutionBoard::new(graph).write(&layout, &feature).unwrap();

        let report = relay(
            &ctx,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "opencode".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(report.value.steps_done, Some(1));
        assert_eq!(report.value.steps_total, Some(3));

        let mut out = Vec::new();
        report.value.write_human(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[2], "plan preserved · 1 of 3 steps done",
            "was: {}",
            lines[2]
        );
        unguard_worktrees(&root);
    }

    /// A board workstream definition, fully defaulted except id and status.
    fn workstream(id: &str, status: WorkstreamStatus) -> WorkstreamDef {
        WorkstreamDef {
            id: id.to_owned(),
            title: id.to_owned(),
            operations: Vec::new(),
            depends_on: Vec::new(),
            write_contract: Vec::new(),
            status,
            provider: None,
            agent: None,
        }
    }

    /// The session record of `session_id` in a hall whose `checkout` feature
    /// holds it.
    fn session_state_of(
        root: &camino::Utf8Path,
        session_id: &str,
    ) -> crate::domain::session::SessionState {
        let layout = Layout::at(root.to_path_buf());
        let session = crate::domain::name::SessionId::new(session_id.to_owned()).unwrap();
        let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session);
        crate::domain::session::SessionState::read(&view_dir)
            .unwrap()
            .unwrap()
    }

    /// Where the view dir's `api` symlink points.
    fn api_link_target(root: &camino::Utf8Path, session_id: &str) -> String {
        let layout = Layout::at(root.to_path_buf());
        let session = crate::domain::name::SessionId::new(session_id.to_owned()).unwrap();
        let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session);
        let link = view_dir.join("api");
        match fs::read_symlink(&link).unwrap() {
            fs::SymlinkTarget::Target(target) => target.to_string(),
            other => panic!("expected a symlink, got {other:?}"),
        }
    }

    #[test]
    fn relay_without_a_previous_session_is_blocked() {
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

        let origins = root.parent().unwrap().join("origins");
        let api_origin = seeded_repo(&origins.join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        feature_create::create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        feature_promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        // No previous session → relay blocked.
        let failure = relay(
            &ctx,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "opencode".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.relay_no_previous");
        unguard_worktrees(&root);
    }

    #[test]
    fn relay_with_same_provider_as_previous_is_blocked() {
        let (_guard, root) = hall_with_provider_session();
        let ctx = Ctx::new(root.clone());

        // Previous session uses claude-code (the default). Try to relay to it.
        let failure = relay(
            &ctx,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "claude-code".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.relay_same_provider");
        unguard_worktrees(&root);
    }

    #[test]
    fn relay_output_when_no_board_exists_shows_zero_of_zero() {
        let (_guard, root) = hall_with_provider_session();
        let ctx = Ctx::new(root.clone());

        let report = relay(
            &ctx,
            RelayInput {
                feature: "checkout".to_owned(),
                provider: "opencode".to_owned(),
            },
        )
        .unwrap();

        let mut out = Vec::new();
        report.value.write_human(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("0 of 0 steps done"),
            "no board → zero of zero, was: {text}"
        );
        unguard_worktrees(&root);
    }
}
