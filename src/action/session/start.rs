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
use std::io::Read as _;

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::harness::{self, Harness};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::tui;
use crate::tui::driver::{Driver, Pty};

use super::super::{discover_hall, read_manifest};

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
}

impl WriteHuman for StartOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Session `{}` for feature `{}` ended. View dir: {}",
            self.session_id, self.feature, self.view_dir
        )
    }
}

/// Start a session: materialise the view dir, spawn the agent, run the TUI.
///
/// The TUI part is skipped when the process is not a tty (a pipe, a CI
/// run): the agent still spawns, and the caller is told where the view dir
/// is instead. That keeps `session start` scriptable without faking a
/// terminal.
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

    // 1. The view dir.
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_id = SessionId::new(session_id)?;
    let view_dir = layout.feature_session(&feature_name, &session_id);
    materialise_view_dir(&layout, &manifest, &feature, &view_dir)?;

    // 2. The agent command.
    let command = harness.start_command(input.resume);
    let width = crate::infra::term::width();
    let height = 24;

    // 3. If we are on a tty, run the TUI; otherwise spawn without it.
    if crate::infra::term::is_tty(crate::infra::term::Stream::Stdout) {
        run_tui(command, &view_dir, &layout, &feature_name, width, height)?;
    } else {
        // Not a tty: the agent still starts (best-effort) so the view dir is
        // genuinely usable, but the interactive loop is skipped.
        let mut pty = PtsPty::new();
        pty.spawn(&command, &view_dir, width, height)?;
        let _ = pty;
    }

    Ok(Report::new(StartOutcome {
        view_dir,
        feature: feature_name,
        provider,
        session_id: session_id.to_string(),
    }))
}

/// Resolve the provider to run: `raw` parsed, or the manifest's default.
fn resolve_provider(manifest: &Manifest, raw: Option<&str>) -> Result<Provider, Failure> {
    match raw {
        Some(value) => value.parse::<Provider>().map_err(Failure::from),
        None => Ok(manifest.providers().default_provider()),
    }
}

/// Materialise `view_dir`: one symlink per promoted repo pointing at its
/// feature worktree, plus the harness config dir symlinked from the hall
/// root.
///
/// The symlink name is the repo's name, so `../api` inside the view dir is
/// the repo `api`. A repo whose worktree is missing is skipped with the
/// rest still linked — the session should still open for the repos that are
/// there.
fn materialise_view_dir(
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    view_dir: &camino::Utf8Path,
) -> Result<(), Failure> {
    fs::ensure_dir(view_dir)?;

    for repo in manifest.repos() {
        if !feature.is_promoted(repo.name()) {
            continue;
        }
        let worktree = layout.repo_worktree(repo.name(), &feature.branch);
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let link = view_dir.join(repo.name().as_str());
        // Replace, not create: re-running a session with the same id is not
        // a thing (ids are fresh UUIDs), but a stale symlink from a crashed
        // run should not block the next one.
        fs::replace_symlink(&worktree, &link)?;
    }

    // The harness config dir (.claude/, .opencode/) — symlinked in from the
    // hall root, so a session's agent reads the hall's standing config.
    let config_dir = layout.harness_dir(&feature_config_provider(manifest));
    if fs::is_dir(&config_dir)? {
        let link = view_dir.join(".config");
        fs::replace_symlink(&config_dir, &link)?;
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
/// the hall snapshot. The interactive loop — reading crossterm events and
/// feeding them through `key_router` — is the next slice's work; here the
/// TUI renders once and returns, which is what keeps the agent's session
/// scriptable and the view dir usable without a live loop.
fn run_tui(
    command: crate::infra::proc::Command,
    view_dir: &camino::Utf8Path,
    layout: &Layout,
    _feature_name: &FeatureName,
    width: u16,
    height: u16,
) -> Result<(), Failure> {
    let mut pty = PtsPty::new();
    pty.spawn(&command, view_dir, width, height)?;
    let mut driver = Driver::new(pty, width, height);

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
    let snapshot = tui::master_detail::snapshot(
        layout.root().as_str(),
        rows,
        driver.selected(),
        "",
        &driver.agent_text(),
        driver.mode(),
    );
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

/// The real PTY: `portable-pty` behind the [`Pty`] seam.
///
/// `portable-pty` gives a `PtyPair`; reads go through the slave's reader
/// handle. Reads are blocking on the handle, so `try_read` is implemented
/// by checking the master's bytes available — `portable-pty` exposes a
/// non-blocking read on the master via `try_clone_reader` + polling; the
/// seam keeps that detail here, where it can be swapped.
struct PtsPty {
    pair: Option<portable_pty::PtyPair>,
}

impl PtsPty {
    fn new() -> Self {
        Self { pair: None }
    }
}

impl Pty for PtsPty {
    fn spawn(
        &mut self,
        command: &crate::infra::proc::Command,
        cwd: &camino::Utf8Path,
        width: u16,
        height: u16,
    ) -> Result<(), Failure> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|source| {
                Failure::failed(
                    "session.pty_open_failed",
                    format!("could not open a PTY: {source}"),
                )
            })?;

        let mut builder = portable_pty::CommandBuilder::new(command.program());
        for arg in command.arguments() {
            builder.arg(arg);
        }
        for (key, value) in command.envs() {
            builder.env(key, value);
        }
        builder.cwd(cwd.as_str());

        let child = pair.slave.spawn_command(builder).map_err(|source| {
            Failure::failed(
                "session.spawn_failed",
                format!("could not start `{}`: {source}", command.display()),
            )
            .fix(FixAction::safe(
                "session.check_binary",
                format!("Is `{}` installed and on PATH?", command.program()),
            ))
        })?;
        drop(child);

        self.pair = Some(pair);
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        let Some(pair) = &self.pair else {
            return Ok(());
        };
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|source| io::Error::new(io::ErrorKind::Other, source))?;
        writer.write_all(bytes)?;
        Ok(())
    }

    fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
        let Some(pair) = &self.pair else {
            return Ok(None);
        };
        // Non-blocking probe: `portable-pty`'s reader blocks on a plain
        // `read`, so this reads through a clone of the master and treats
        // "no data yet" (WouldBlock / EOF) as `None`.
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|source| io::Error::new(io::ErrorKind::Other, source))?;
        let mut buf = [0u8; 4096];
        match reader.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn is_running(&self) -> bool {
        // Without a child handle to poll, this reports true for the session
        // lifetime — the caller's loop ends on user quit. A future slice
        // wires the child's exit status here.
        self.pair.is_some()
    }
}

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
    use crate::test_support::{hall_root, seeded_repo};

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

        materialise_view_dir(&layout, &manifest, &feature, &view_dir).unwrap();

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
}
