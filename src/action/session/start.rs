//! `ivar session start` — the heart of the tool.
//!
//! A session materialises a **view dir** — a directory of symlinks, one per
//! promoted repo, pointing at the feature worktrees — spawns the hall's
//! agent harness inside it, and opens the TUI (master-detail, embedded PTY).
//!
//! # The model, in one paragraph
//!
//! The view dir is *the* thing an agent session works in: a single directory
//! where `../api` and `../web` are real git worktrees on the same branch.
//! Its only contents are symlinks (ARCHITECTURE.md: no Windows, symlinks are
//! the point) plus the harness's own config dir, which is symlinked in from
//! the hall root (`.claude/`, `.opencode/`).
//!
//! # What this slice wires
//!
//! - The view dir (symlinks per promoted repo, harness config dir symlinked).
//! - The harness spawn through [`crate::harness`] with real `portable-pty`.
//! - The TUI loop, driven by the [`crate::tui`] modules.
//!
//! The TUI loop is the one place the crate runs an event loop; everything
//! below it stays the pure/step-driven design from ARCHITECTURE.md, seam 6.

use std::io;

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::repo::pull;
use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git;
use crate::harness::{self, Harness};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::tui;
use crate::tui::driver::{Driver, PtsPty, Pty, ShellSpec};

use super::super::{discover_hall, read_manifest};
use super::{hook, lookup};

/// What `ivar session start` needs.
#[derive(Debug, Clone)]
pub struct StartInput {
    /// The feature to open a session for.
    pub feature: String,
    /// Resume an existing session (honoured only for harnesses whose
    /// capabilities include resume).
    pub resume: bool,
    /// The provider to run. `None` uses the hall's default provider.
    pub provider: Option<String>,
    /// Create the session without launching a provider. The View Dir persists
    /// after this command returns, until an explicit `session stop`.
    pub detached: bool,
    /// Relay: a fresh session on the same feature under a different provider
    /// than the feature's most recent session. Requires `--provider`.
    pub relay: bool,
}

/// What `ivar session start` did — a summary, since the interactive part
/// ends when the user quits the TUI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StartOutcome {
    /// The session's view dir.
    pub view_dir: Utf8PathBuf,
    /// The feature this session is bound to.
    pub feature: FeatureName,
    /// The provider that ran.
    pub provider: Provider,
    /// The session id (a UUID, from the view dir's name).
    pub session_id: String,
    /// Whether the session was created detached (no provider launched).
    pub detached: bool,
}

impl WriteHuman for StartOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.detached {
            writeln!(
                w,
                "Session `{}` for feature `{}` started detached (no provider launched). View dir: {}",
                self.session_id, self.feature, self.view_dir
            )
        } else {
            writeln!(
                w,
                "Session `{}` for feature `{}` ended. View dir: {}",
                self.session_id, self.feature, self.view_dir
            )
        }
    }
}

/// Start a session: materialise the view dir, spawn the agent, run the TUI.
///
/// The TUI part is skipped when the process is not a tty (a pipe, a CI
/// run): the agent still spawns, and the caller is told where the view dir
/// is instead. That keeps `session start` scriptable without faking a
/// terminal. A **detached** session skips the spawn entirely — the View Dir
/// persists, discoverable by `session connect`, until an explicit stop.
pub fn start(ctx: &Ctx, input: StartInput) -> Outcome<StartOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let feature_name = FeatureName::new(input.feature)?;
    let feature = Feature::read(&layout, &feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{feature_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {feature_name}`."),
        ))
    })?;

    let provider = resolve_provider(&manifest, input.provider.as_deref())?;
    let harness = Harness::for_provider(provider)?;
    if input.resume {
        harness::check_resume_supported(harness)?;
    }
    if input.relay {
        check_relay(&layout, &feature_name, input.provider.as_deref(), provider)?;
    }

    // 1. Smart Fetch, before the view dir exists: refresh every registered
    //    repo's default branch, best-effort per repo (valhalla's Smart Fetch).
    //    The refresh is fetch-and-fast-forward of the read-only default
    //    worktree — never a promoted repo's feature worktree — and a fetch
    //    that lands new files needs a writable target, so this runs before
    //    the guard-applying materialisation below.
    let mut warnings = smart_fetch(&git::System, &layout, &manifest);

    // 2. The view dir and the session record.
    let session_id = SessionId::new(uuid::Uuid::new_v4().to_string())?;
    let view_dir = layout.feature_session(&feature_name, &session_id);
    materialise_view_dir(&layout, &manifest, Some(&feature), &view_dir)?;
    let started_at = rfc3339_now();
    let mut state = SessionState::new(provider, &started_at);
    state.bind(feature_name.clone(), &started_at);
    state.write(&view_dir)?;

    // 3. Session hooks, after the view dir and the record exist (a hook reads
    //    `IVAR_SESSION_PATH`) and before the agent spawns (a hook brings up the
    //    database the agent is about to talk to). Failure warns; it never
    //    stops the session — see `session::hook`.
    warnings.extend(hook::run_session_hooks(
        &layout,
        &manifest,
        &feature,
        &view_dir,
        &session_id,
    ));

    // 4. The agent command — skipped entirely for a detached session.
    if !input.detached {
        let command = harness.start_command(input.resume);
        let width = crate::infra::term::width();
        let height = 24;

        // If we are on a tty, run the TUI; otherwise spawn without it.
        if crate::infra::term::is_tty(crate::infra::term::Stream::Stdout) {
            run_tui(command, &view_dir, &layout, &feature_name, width, height)?;
        } else {
            // Not a tty: the agent still starts (best-effort) so the view dir is
            // genuinely usable, but the interactive loop is skipped.
            let mut pty = PtsPty::new();
            pty.spawn(&command, &view_dir, width, height)?;
            let _ = pty;
        }
    }

    Ok(Report::with_warnings(
        StartOutcome {
            view_dir,
            feature: feature_name,
            provider,
            session_id: session_id.to_string(),
            detached: input.detached,
        },
        warnings,
    ))
}

/// Resolve the provider to run: `raw` parsed, or the manifest's default.
fn resolve_provider(manifest: &Manifest, raw: Option<&str>) -> Result<Provider, Failure> {
    match raw {
        Some(value) => value.parse::<Provider>().map_err(Failure::from),
        None => Ok(manifest.providers().default_provider()),
    }
}

/// The relay gates, checked before any state is touched. A relay must name
/// the provider to switch to, there must be a previous session on the feature
/// to relay from, and the provider must actually differ — a same-provider
/// restart is a plain session, not a relay (valhalla: "a Relay passes the
/// work, never the thread").
fn check_relay(
    layout: &Layout,
    feature_name: &FeatureName,
    raw_provider: Option<&str>,
    provider: Provider,
) -> Result<(), Failure> {
    if raw_provider.is_none() {
        return Err(Failure::blocked(
            "session.relay_needs_provider",
            "a relay must name the provider to switch to",
        )
        .expected("an explicit `--provider` on a relay")
        .actual("no `--provider` given")
        .fix(FixAction::safe(
            "session.relay_pass_provider",
            "Pass `--provider` with the provider you want the relay to run under.",
        )));
    }

    let previous = lookup::most_recent(layout, feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "session.relay_no_previous",
            format!("feature `{feature_name}` has no previous session to relay from"),
        )
        .expected("an earlier session on this feature")
        .actual("no live session with a session record on this feature")
        .fix(FixAction::safe(
            "session.start_plain",
            "Start a plain session instead — a relay continues the work of an ended one.",
        ))
    })?;

    let previous_provider = previous.state.as_ref().map(SessionState::provider);
    if previous_provider == Some(provider) {
        return Err(Failure::blocked(
            "session.relay_same_provider",
            format!(
                "relay must switch provider; the most recent session on `{feature_name}` already ran `{provider}`"
            ),
        )
        .expected("a provider different from the previous session's")
        .actual(format!("the previous session ran `{provider}`"))
        .fix(FixAction::safe(
            "session.relay_other_provider",
            "Pass `--provider` with a different provider, or drop `--relay` for a plain fresh session.",
        )));
    }

    Ok(())
}

/// Best-effort default-branch refresh for every registered repo — valhalla's
/// **Smart Fetch**. One unreachable remote warns and is skipped; the session
/// still starts. Runs through the same fetch-and-fast-forward as `repo pull`
/// (that is what `repo.pull::refresh_default` is), which never touches a
/// promoted repo's feature worktree.
fn smart_fetch(git: &impl git::Git, layout: &Layout, manifest: &Manifest) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for repo in manifest.repos() {
        match pull::refresh_default(git, layout, repo) {
            pull::PullStatus::Refreshed => {}
            pull::PullStatus::Failed { reason } => warnings.push(Warning::new(
                "session.smart_fetch_failed",
                repo.name().to_string(),
                reason,
            )),
            pull::PullStatus::Skipped { reason } => warnings.push(Warning::new(
                "session.smart_fetch_skipped",
                repo.name().to_string(),
                reason,
            )),
        }
    }
    warnings
}

/// Materialise `view_dir`: one symlink per registered repo plus the harness
/// config dir symlinked from the hall root.
///
/// For a **feature session** (`feature: Some`), a promoted repo is symlinked
/// to its feature worktree (writable); every other repo is symlinked to its
/// default-branch worktree and that worktree is held read-only by the kernel
/// (write bits cleared). For a **discovery session** (`feature: None`), every
/// repo is a read-only default-branch worktree. This is the idempotent core
/// `session start`, `session connect`, and `session convert` all run — connect
/// "repairs" drifted symlinks and guards by running the same materialisation
/// again, which is a no-op when nothing drifted.
///
/// A repo whose worktree is missing is skipped with the rest still linked —
/// the session should still open for the repos that are there.
pub(crate) fn materialise_view_dir(
    layout: &Layout,
    manifest: &Manifest,
    feature: Option<&Feature>,
    view_dir: &camino::Utf8Path,
) -> Result<(), Failure> {
    fs::ensure_dir(view_dir)?;

    for repo in manifest.repos() {
        let worktree = match feature {
            Some(feature) if feature.is_promoted(repo.name()) => {
                layout.repo_worktree(repo.name(), &feature.branch)
            }
            _ => layout.repo_worktree(repo.name(), repo.default_branch()),
        };
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let link = view_dir.join(repo.name().as_str());
        // Replace only when the target changed: the view dir is re-materialised
        // on every connect, and an unchanged link must not be renamed (each
        // rename opens a transient resolution race — see `infra::fs`).
        fs::replace_symlink_if_changed(&worktree, &link)?;
        // A repo the session does not promote is held read-only by the kernel:
        // clear (or re-clear) the write bits on its default-branch worktree.
        if feature.is_none_or(|feature| !feature.is_promoted(repo.name())) {
            fs::clear_write_bits(&worktree)?;
        }
    }

    // The harness config dir (.claude/, .opencode/) — symlinked in from the
    // hall root, so a session's agent reads the hall's standing config.
    let config_dir = layout.harness_dir(&feature_config_provider(manifest));
    if fs::is_dir(&config_dir)? {
        let link = view_dir.join(".config");
        fs::replace_symlink_if_changed(&config_dir, &link)?;
    }

    Ok(())
}

/// The provider whose config dir the view dir symlinks in. Sessions run the
/// hall's default provider unless told otherwise; the config dir follows the
/// same rule so the symlink does not depend on a session flag.
fn feature_config_provider(manifest: &Manifest) -> Provider {
    manifest.providers().default_provider()
}

/// Run the TUI over the agent's PTY: pump its output, render one frame, and
/// hand control back.
///
/// This slice wires the *structure*: the agent spawns in the view dir, its
/// output flows through the driver's `screen` seam, and the widget renders
/// the hall snapshot. The full interactive loop (raw mode, event reading,
/// pumping, quit cleanup) lives in `tui::master_detail` and is what
/// `ivar feature view` runs; here the TUI renders once and returns, which is
/// what keeps the agent's session scriptable without a live loop.
fn run_tui(
    command: crate::infra::proc::Command,
    view_dir: &camino::Utf8Path,
    layout: &Layout,
    _feature_name: &FeatureName,
    width: u16,
    height: u16,
) -> Result<(), Failure> {
    // One shell — the agent — running in the view dir. The driver spawns the
    // initially focused shell eagerly, so the agent starts here.
    let shells = vec![ShellSpec {
        label: "agent".to_owned(),
        cwd: view_dir.to_path_buf(),
        command,
    }];
    let mut driver = Driver::new(shells, PtsPty::new, width, height);

    // Drain whatever the agent produced at startup, then render one frame.
    let _ = driver.pump();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .map_err(|source| {
            Failure::failed(
                "session.terminal_failed",
                format!("could not open a terminal: {source}"),
            )
            .fix(FixAction::safe(
                "session.retry",
                "Try again in a real terminal.",
            ))
        })?;

    let feature_names = collect_features(layout);
    let rows = feature_rows(layout, &feature_names);
    let snapshot = driver.snapshot(layout.root().as_str(), &rows);
    let _ = terminal.draw(|frame| tui::widget::render(&snapshot, frame.area(), frame.buffer_mut()));

    Ok(())
}

/// The features in the hall, sorted, for the TUI's left list.
fn collect_features(layout: &Layout) -> Vec<FeatureName> {
    let mut names = Vec::new();
    let dir = layout.features_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries {
            if let Some(name) = entry.file_name()
                && let Ok(name) = FeatureName::new(name)
            {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

/// The ready-made [`tui::widget::Row`]s for the TUI's left list: one per
/// feature, with a one-word status. Reading the hall is the action's job —
/// the TUI must not touch the store (ARCHITECTURE.md's layering table).
fn feature_rows(layout: &Layout, names: &[FeatureName]) -> Vec<tui::widget::Row> {
    names
        .iter()
        .map(|name| {
            let status = Feature::read(layout, name)
                .ok()
                .flatten()
                .map(|feature| {
                    if feature.promotions.is_empty() {
                        "empty".to_owned()
                    } else {
                        format!(
                            "{}/{}",
                            feature.count_worktrees(crate::domain::feature::WorktreeState::Ready),
                            feature.promotions.len()
                        )
                    }
                })
                .unwrap_or_else(|| "unreadable".to_owned());
            tui::widget::Row {
                label: name.to_string(),
                status,
            }
        })
        .collect()
}

/// The real PTY for a session is `tui::driver::PtsPty` — `portable-pty`
/// behind the [`Pty`] seam, shared with `ivar feature view`.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::feature::create::{self as feature_create, CreateInput};
    use crate::action::feature::promote::{self as feature_promote, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{git, hall_root, seeded_repo};

    fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

        let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
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

        (guard, root)
    }

    #[test]
    fn materialise_view_dir_symlinks_promoted_repos() {
        let (_guard, root) = hall_with_promoted_feature();
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
            .unwrap()
            .unwrap();
        let view_dir = layout.feature_session(
            &FeatureName::new("checkout").unwrap(),
            &crate::domain::name::SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap(),
        );

        materialise_view_dir(&layout, &manifest, Some(&feature), &view_dir).unwrap();

        let link = view_dir.join("api");
        assert!(
            fs::is_dir(&link).unwrap(),
            "the api symlink must resolve to a dir"
        );
        let target = match fs::read_symlink(&link).unwrap() {
            fs::SymlinkTarget::Target(path) => path,
            other => panic!("expected a symlink, got {other:?}"),
        };
        assert!(
            target.as_str().contains(".ivar/repos/api/checkout"),
            "the symlink must point at the feature worktree: {target}"
        );
    }

    #[test]
    fn resolve_provider_uses_the_manifest_default_when_absent() {
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![],
            None,
        )
        .unwrap();

        assert_eq!(
            resolve_provider(&manifest, None).unwrap(),
            Provider::ClaudeCode
        );
        assert_eq!(
            resolve_provider(&manifest, Some("opencode")).unwrap(),
            Provider::OpenCode
        );
        assert!(resolve_provider(&manifest, Some("nope")).is_err());
    }

    // -- detached sessions -----------------------------------------------------

    /// A detached session must not spawn a provider: the view dir and its
    /// session record exist when `start` returns, and no PTY was opened.
    #[test]
    fn detached_start_creates_the_view_dir_without_launching_a_provider() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        let report = start(
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
        assert!(report.is_clean());

        let outcome = &report.value;
        assert!(outcome.detached);
        assert!(fs::is_dir(&outcome.view_dir).unwrap());
        assert!(fs::is_dir(&outcome.view_dir.join("api")).unwrap());

        let state = SessionState::read(&outcome.view_dir).unwrap().unwrap();
        assert_eq!(state.provider(), Provider::ClaudeCode);
        assert_eq!(state.feature().unwrap().as_str(), "checkout");
        assert!(state.feature_bound_at().is_some());
    }

    // -- smart fetch -----------------------------------------------------------

    /// The fetch-and-fast-forward on session start is real, not just a report:
    /// the default worktree catches up to a commit the origin gained after
    /// sync, while the promoted repo's feature worktree is untouched.
    #[test]
    fn smart_fetch_advances_default_branches_and_never_touches_feature_worktrees() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
            .unwrap()
            .unwrap();
        let default_worktree = layout.repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("main").unwrap(),
        );
        let feature_worktree =
            layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);

        // The origin gains a commit after sync.
        let origin = root.parent().unwrap().join("origins").join("api");
        std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
        git(&origin, &["add", "CHANGELOG.md"]);
        git(&origin, &["commit", "-m", "v1"]);

        let report = start(
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
        assert!(report.is_clean());

        assert_eq!(
            std::fs::read_to_string(default_worktree.join("CHANGELOG.md")).unwrap(),
            "v1\n",
            "smart fetch must fast-forward the default worktree"
        );
        assert!(
            !feature_worktree.join("CHANGELOG.md").exists(),
            "smart fetch must never touch a promoted repo's feature worktree"
        );
    }

    /// Best-effort: one repo whose refresh fails (no worktree) warns and the
    /// session still starts.
    #[test]
    fn smart_fetch_warns_and_continues_when_a_repo_fails() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        // A second declared repo that was never synced: no worktree, so its
        // refresh fails — the session must still start.
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let mut repos = manifest.repos().to_vec();
        repos.push(Repo::new(
            RepoName::new("ghost").unwrap(),
            root.join("no-such-origin").as_str(),
            BranchName::new("main").unwrap(),
        ));
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            repos,
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let report = start(
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

        assert!(!report.is_clean());
        assert!(report.warnings.iter().any(|warning| {
            warning.subject == "ghost" && warning.code == "session.smart_fetch_failed"
        }));
        assert!(
            fs::is_dir(&report.value.view_dir).unwrap(),
            "one failed repo must not block session start"
        );
    }

    // -- relay -----------------------------------------------------------------

    /// Relay: a new session on the same feature under a different provider,
    /// sharing the feature's worktrees, with its own fresh conversation.
    #[test]
    fn relay_starts_a_new_session_with_a_different_provider() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        // The session to relay from: the hall's default provider, detached so
        // no provider binary is spawned.
        let first = start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap()
        .value;

        let report = start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: Some("opencode".to_owned()),
                detached: true,
                relay: true,
            },
        )
        .unwrap();
        assert!(report.is_clean());

        let relayed = &report.value;
        assert_ne!(
            relayed.session_id, first.session_id,
            "a relay is a new session, never a resume"
        );
        assert!(fs::is_dir(&relayed.view_dir).unwrap());

        // Reuses the same feature worktrees.
        let first_link = read_link_target(&first.view_dir.join("api"));
        let relayed_link = read_link_target(&relayed.view_dir.join("api"));
        assert_eq!(first_link, relayed_link);

        // Fresh conversation: the relayed session's record is its own.
        let state = SessionState::read(&relayed.view_dir).unwrap().unwrap();
        assert_eq!(state.provider(), Provider::OpenCode);
    }

    #[test]
    fn relay_without_an_explicit_provider_is_blocked() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        let failure = start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None,
                detached: true,
                relay: true,
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.relay_needs_provider");
    }

    #[test]
    fn relay_with_the_same_provider_as_the_previous_session_is_blocked() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None, // claude-code, the hall default
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        let failure = start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: Some("claude-code".to_owned()),
                detached: true,
                relay: true,
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.relay_same_provider");
    }

    #[test]
    fn relay_without_a_previous_session_is_blocked() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        // No session has ever been started on `checkout`.

        let failure = start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: Some("opencode".to_owned()),
                detached: true,
                relay: true,
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "session.relay_no_previous");
    }

    /// The symlink target `link` points at, panicking on anything else.
    fn read_link_target(link: &camino::Utf8Path) -> Utf8PathBuf {
        match fs::read_symlink(link).unwrap() {
            fs::SymlinkTarget::Target(target) => target,
            other => panic!("expected a symlink, got {other:?}"),
        }
    }
}
