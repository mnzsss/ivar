//! `ivar session convert` — one-way conversion of a discovery session into a
//! feature session.
//!
//! Valhalla's **Session Conversion**: bind a discovery session (no feature)
//! to an existing feature, moving its View Dir from `.sessions/<id>/` to
//! `.features/<feature>/sessions/<id>/` and rebuilding the symlinks for the
//! target feature. The Session ID, provider, and original `started_at` are
//! preserved — the session's `state.json` moves with the directory.
//!
//! # The transition state
//!
//! Conversion is not atomic, so an interrupted run must not leave the session
//! in an ambiguous half-moved condition. Before any step runs, a `.converting`
//! marker is written under the destination feature's directory
//! (`.features/<feature>/.converting`) naming the session, its source path,
//! and the feature. On retry, the marker is detected first and the conversion
//! is resumed: every step is idempotent and re-derived from disk, so "resume
//! from the last completed step" falls out of re-running them.
//!
//! Once converted, a session can never revert to discovery: a second convert
//! of the same session is refused.

use std::io;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, json};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::lookup;
use super::start::materialise_view_dir;
use crate::action::Ctx;

/// What `ivar session convert` needs.
#[derive(Debug, Clone)]
pub struct ConvertInput {
    /// The discovery session's id, or a unique prefix of one.
    pub session_id: String,
    /// The feature to bind the session to. Must already exist.
    pub feature: String,
}

/// What `ivar session convert` did.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertOutcome {
    /// The session's id — unchanged by conversion.
    pub session_id: String,
    /// The feature the session is now bound to.
    pub feature: FeatureName,
    /// The session's new view dir, under the feature's session tree.
    pub view_dir: Utf8PathBuf,
}

impl WriteHuman for ConvertOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Converted session `{}` to feature `{}`. View dir: {}",
            self.session_id, self.feature, self.view_dir
        )
    }
}

/// The marker file's name, under the destination feature's directory.
const CONVERTING_FILE: &str = ".converting";

/// The transition record of an in-flight conversion.
///
/// `step` is not used to drive resume — disk state is the truth, and every
/// step below is idempotent — but it documents how far the interrupted run
/// got, which is what a human reading `.converting` wants to see.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition {
    /// The session being converted.
    session_id: SessionId,
    /// The View Dir's path before the move — where resume looks for the
    /// session first (it may already be gone if the move completed).
    source: Utf8PathBuf,
    /// The feature the session is being bound to.
    feature: FeatureName,
    /// How far the interrupted run got.
    step: Step,
}

/// Which phase an in-flight conversion is in. Diagnostic only — see
/// [`Transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    /// About to move the View Dir.
    MoveSession,
    /// About to bind the session state.
    UpdateState,
    /// About to re-materialise the View Dir for the target feature.
    Rematerialize,
}

/// Convert a discovery session into a feature session.
///
/// `Blocked` when the session is not a discovery session (already converted),
/// when the destination feature does not exist, or when the session cannot be
/// located. `Failed` when a step breaks mid-flight — the `.converting` marker
/// then lets the next attempt resume.
pub fn convert(ctx: &Ctx, input: ConvertInput) -> Outcome<ConvertOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let feature_name = FeatureName::new(input.feature)?;

    // 1. An interrupted conversion for this feature? Resume it first — the
    //    session may already have moved, so the discovery checks below would
    //    misjudge it. The marker wins over the request (bifrost does the
    //    same): a pending conversion of this feature is resumed as-is.
    if let Some(transition) = read_transition(&layout, &feature_name)? {
        return resume(&layout, &manifest, &feature_name, transition);
    }

    // 2. Locate the session and verify it is a discovery session.
    let session = lookup::resolve(&layout, Some(&input.session_id), None)?;
    let state = session.state.as_ref().ok_or_else(|| {
        Failure::blocked(
            "session.state_missing",
            format!("session `{}` has no session record", session.id),
        )
        .expected("a session with a `state.json` in its view dir")
        .actual("the view dir exists but no state.json does")
        .fix(FixAction::safe(
            "session.start_fresh",
            "Start a fresh session instead — conversion needs the session's record.",
        ))
    })?;
    if session.feature.is_some() || !state.is_discovery() {
        return Err(Failure::blocked(
            "session.convert_already_bound",
            format!("session `{}` is already bound to a feature", session.id),
        )
        .expected("a discovery session (no feature bound)")
        .actual(format!(
            "the session is bound to `{}`",
            session
                .feature
                .as_ref()
                .map_or("an unknown feature", |f| f.as_str())
        ))
        .fix(FixAction::safe(
            "session.convert_once",
            "Conversion is one-way; a bound session cannot be converted again.",
        )));
    }

    // 3. The destination feature must exist.
    let feature = Feature::read(&layout, &feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("an existing feature to bind the session to")
        .actual(format!("`{feature_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {feature_name}`."),
        ))
    })?;

    // 4. Record the transition, then run the (idempotent, resumable) steps.
    let transition = Transition {
        session_id: session.id.clone(),
        source: session.view_dir.clone(),
        feature: feature_name.clone(),
        step: Step::MoveSession,
    };
    write_transition(&layout, &feature_name, &transition)?;
    run_conversion(&layout, &manifest, &feature_name, &feature, transition)
}

/// Resume an interrupted conversion. The marker's record is authoritative —
/// the session's location on disk decides which steps still need to run.
fn resume(
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    transition: Transition,
) -> Outcome<ConvertOutcome> {
    // The destination feature must still exist — rematerialising a view dir
    // needs its promotion record.
    let feature = Feature::read(layout, feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("the feature an interrupted conversion was targeting")
        .actual("its feature.json is gone")
        .fix(FixAction::safe(
            "feature.recreate",
            format!("Recreate the feature with `ivar feature create {feature_name}`, then retry."),
        ))
    })?;
    run_conversion(layout, manifest, feature_name, &feature, transition)
}

/// The conversion steps, in order. Each is idempotent and re-derived from
/// disk, so a retry after any interruption picks up exactly where the last
/// run stopped.
fn run_conversion(
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    feature: &Feature,
    mut transition: Transition,
) -> Outcome<ConvertOutcome> {
    let dest = layout.feature_session(feature_name, &transition.session_id);

    // Step 1 — move the View Dir into the feature's session tree. Resume
    // handles the crash-after-move case: the source is gone and the
    // destination exists, so there is nothing to move.
    if transition.step == Step::MoveSession {
        match (fs::is_dir(&transition.source)?, fs::is_dir(&dest)?) {
            (true, _) => {
                let Some(parent) = dest.parent() else {
                    return Err(Failure::failed(
                        "session.convert_no_parent",
                        format!("`{dest}` has no parent directory"),
                    ));
                };
                fs::ensure_dir(parent)?;
                fs::rename(&transition.source, &dest)?;
            }
            (false, true) => {}
            (false, false) => {
                return Err(Failure::failed(
                    "session.convert_missing_view_dir",
                    format!(
                        "the session's view dir is neither at `{}` nor `{dest}`",
                        transition.source
                    ),
                )
                .expected("the session's view dir, before or after the move")
                .actual("both paths are absent")
                .fix(FixAction::safe(
                    "session.start_fresh",
                    "Start a fresh session — this one's view dir cannot be recovered.",
                )));
            }
        }
        transition.step = Step::UpdateState;
        write_transition(layout, feature_name, &transition)?;
    }

    // Step 2 — bind the session's record to the feature. `started_at` and
    // `provider` were carried along by the move and are preserved untouched.
    if transition.step == Step::UpdateState {
        let mut state = SessionState::read(&dest)?.ok_or_else(|| {
            Failure::blocked(
                "session.state_missing",
                format!("session `{}` has no session record", transition.session_id),
            )
            .expected("a session with a `state.json` in its view dir")
            .actual("the view dir exists but no state.json does")
            .fix(FixAction::safe(
                "session.start_fresh",
                "Start a fresh session instead — conversion needs the session's record.",
            ))
        })?;
        state.bind(feature_name.clone(), rfc3339_now());
        state.write(&dest)?;
        transition.step = Step::Rematerialize;
        write_transition(layout, feature_name, &transition)?;
    }

    // Step 3 — rebuild the View Dir for the target feature, and clear the
    // transition: the conversion is complete.
    if transition.step == Step::Rematerialize {
        materialise_view_dir(layout, manifest, Some(feature), &dest)?;
        fs::remove_file(&transition_path(layout, feature_name))?;
    }

    Ok(Report::new(ConvertOutcome {
        session_id: transition.session_id.to_string(),
        feature: feature_name.clone(),
        view_dir: dest,
    }))
}

/// `.features/<feature>/.converting` — the transition marker for that feature.
fn transition_path(layout: &Layout, feature: &FeatureName) -> Utf8PathBuf {
    layout.feature_dir(feature).join(CONVERTING_FILE)
}

/// Read the transition marker for `feature`. `Ok(None)` when none is pending.
fn read_transition(layout: &Layout, feature: &FeatureName) -> Result<Option<Transition>, Failure> {
    json::read(&transition_path(layout, feature)).map_err(Failure::from)
}

/// Write the transition marker for `feature`, atomically, in canonical form.
fn write_transition(
    layout: &Layout,
    feature: &FeatureName,
    transition: &Transition,
) -> Result<(), Failure> {
    json::write_canonical(&transition_path(layout, feature), transition).map_err(Failure::from)
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
    use crate::action::feature::create::{self as feature_create, CreateInput};
    use crate::action::feature::promote::{self as feature_promote, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    const DISCOVERY_ID: &str = "2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c";
    const STARTED_AT: &str = "2026-01-01T00:00:00.000000000Z";

    /// A hall with `api` promoted into `checkout`, and a discovery session
    /// whose view dir materialises every repo read-only.
    fn hall_with_discovery_session() -> (tempfile::TempDir, Utf8PathBuf) {
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
        let web_origin = seeded_repo(&origins.join("web"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![
                Repo::new(
                    RepoName::new("api").unwrap(),
                    api_origin.as_str(),
                    BranchName::new("main").unwrap(),
                ),
                Repo::new(
                    RepoName::new("web").unwrap(),
                    web_origin.as_str(),
                    BranchName::new("main").unwrap(),
                ),
            ],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        feature_create::create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
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

        (guard, root)
    }

    /// Materialise a discovery view dir with a session record, as a session
    /// created outside ivar (or before session records existed) would leave.
    fn discovery_view_dir(layout: &Layout) -> Utf8PathBuf {
        let session_id = SessionId::new(DISCOVERY_ID).unwrap();
        let view_dir = layout.discovery_session(&session_id);
        let manifest = Manifest::read(layout).unwrap().unwrap();
        materialise_view_dir(layout, &manifest, None, &view_dir).unwrap();
        let state = SessionState::new(Provider::ClaudeCode, STARTED_AT);
        state.write(&view_dir).unwrap();
        view_dir
    }

    /// Undo the read-only guards materialisation applied, so the TempDir can
    /// clean up after the test.
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

    fn feature_name() -> FeatureName {
        FeatureName::new("checkout").unwrap()
    }

    #[test]
    fn convert_moves_the_view_dir_and_rebuilds_symlinks() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let old_dir = discovery_view_dir(&layout);
        assert!(fs::is_dir(&old_dir).unwrap());

        let report = convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let session_id = SessionId::new(DISCOVERY_ID).unwrap();
        let new_dir = layout.feature_session(&feature_name(), &session_id);
        assert_eq!(report.value.view_dir, new_dir);
        assert!(
            !fs::is_dir(&old_dir).unwrap(),
            "the discovery view dir must move"
        );
        assert!(fs::is_dir(&new_dir).unwrap());

        // Symlinks rebuilt for the target feature: api → feature worktree,
        // web → read-only default worktree.
        let api_target = match fs::read_symlink(&new_dir.join("api")).unwrap() {
            fs::SymlinkTarget::Target(path) => path,
            other => panic!("expected a symlink, got {other:?}"),
        };
        assert!(
            api_target.as_str().contains(".ivar/repos/api/checkout"),
            "api must point at the feature worktree: {api_target}"
        );
        let web_target = match fs::read_symlink(&new_dir.join("web")).unwrap() {
            fs::SymlinkTarget::Target(path) => path,
            other => panic!("expected a symlink, got {other:?}"),
        };
        assert!(
            web_target.as_str().contains(".ivar/repos/web/main"),
            "web must point at the read-only default worktree: {web_target}"
        );

        // The transition marker is gone.
        assert!(!fs::exists(&transition_path(&layout, &feature_name())).unwrap());
        unguard_worktrees(&root);
    }

    /// Conversion preserves the session id, provider, and original
    /// `started_at` — the state file moves with the directory, unchanged
    /// except for the binding.
    #[test]
    fn convert_preserves_session_id_provider_and_started_at() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        discovery_view_dir(&layout);

        let report = convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(report.value.session_id, DISCOVERY_ID);
        let session_id = SessionId::new(DISCOVERY_ID).unwrap();
        let state = SessionState::read(&layout.feature_session(&feature_name(), &session_id))
            .unwrap()
            .unwrap();
        assert_eq!(state.provider(), Provider::ClaudeCode);
        assert_eq!(state.started_at(), STARTED_AT);
        assert_eq!(state.feature().unwrap().as_str(), "checkout");
        assert!(state.feature_bound_at().is_some());
        unguard_worktrees(&root);
    }

    /// Conversion is one-way: converting an already-converted session is
    /// refused.
    #[test]
    fn convert_refuses_an_already_converted_session() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        discovery_view_dir(&layout);
        convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let failure = convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.convert_already_bound");
        unguard_worktrees(&root);
    }

    /// Converting a session that is already a feature session (started
    /// directly on a feature) is refused the same way.
    #[test]
    fn convert_refuses_a_feature_session() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        discovery_view_dir(&layout);

        // A feature session: view dir under the feature + bound state.
        let feature_session_id = SessionId::new("3d7f7f2e-3e9b-4c4a-8d3b-7b8f8f0a2c3d").unwrap();
        let feature_dir = layout.feature_session(&feature_name(), &feature_session_id);
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let feature = Feature::read(&layout, &feature_name()).unwrap().unwrap();
        materialise_view_dir(&layout, &manifest, Some(&feature), &feature_dir).unwrap();
        let mut state = SessionState::new(Provider::ClaudeCode, STARTED_AT);
        state.bind(feature_name(), STARTED_AT);
        state.write(&feature_dir).unwrap();

        let failure = convert(
            &ctx,
            ConvertInput {
                session_id: "3d7f7f2e".to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.convert_already_bound");
        unguard_worktrees(&root);
    }

    #[test]
    fn convert_refuses_a_missing_feature() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        discovery_view_dir(&layout);

        let failure = convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "ghost".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "feature.not_found");
        unguard_worktrees(&root);
    }

    /// The transition marker prevents double-conversion: once it exists, a
    /// retry resumes the recorded conversion instead of starting a fresh one
    /// (the marker wins over the request, exactly as bifrost does).
    #[test]
    fn an_interrupted_conversion_resumes_on_retry() {
        let (_guard, root) = hall_with_discovery_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let old_dir = discovery_view_dir(&layout);

        // Simulate a run interrupted after the move but before the state
        // update: the view dir is already in the feature tree, and the marker
        // says so.
        let session_id = SessionId::new(DISCOVERY_ID).unwrap();
        let dest = layout.feature_session(&feature_name(), &session_id);
        let Some(parent) = dest.parent() else {
            panic!("no parent");
        };
        fs::ensure_dir(parent).unwrap();
        fs::rename(&old_dir, &dest).unwrap();
        let transition = Transition {
            session_id: session_id.clone(),
            source: old_dir.clone(),
            feature: feature_name(),
            step: Step::MoveSession,
        };
        write_transition(&layout, &feature_name(), &transition).unwrap();

        // Retry: resumes, completing the move bookkeeping, the state bind,
        // and the re-materialisation.
        let report = convert(
            &ctx,
            ConvertInput {
                session_id: DISCOVERY_ID.to_owned(),
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(report.value.session_id, DISCOVERY_ID);
        assert!(fs::is_dir(&dest).unwrap());
        let state = SessionState::read(&dest).unwrap().unwrap();
        assert_eq!(state.provider(), Provider::ClaudeCode);
        assert_eq!(state.started_at(), STARTED_AT);
        assert_eq!(state.feature().unwrap().as_str(), "checkout");
        assert!(!fs::exists(&transition_path(&layout, &feature_name())).unwrap());
        unguard_worktrees(&root);
    }
}
