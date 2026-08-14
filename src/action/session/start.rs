//! `ivar session start` — the heart of the tool.
//!
//! A session materialises a **view dir** — a directory of symlinks, one per
//! promoted repo, pointing at the feature worktrees, plus a real per-session
//! harness config dir — spawns the hall's agent harness inside it, and opens
//! the TUI (master-detail, embedded PTY).
//!
//! # The model, in one paragraph
//!
//! The view dir is *the* thing an agent session works in: a single directory
//! where `../api` and `../web` are real git worktrees on the same branch.
//! Every repo entry is a symlink (ARCHITECTURE.md: no Windows, symlinks are
//! the point), but the harness's own config dir (`.claude/`, `.opencode/`) is
//! a **real directory**, not a symlink to the hall's — only its `commands/`
//! subdirectory is symlinked back to the hall, so the hall's shipped
//! `/ivar-*` commands reach the agent without a session's config writes
//! landing in the hall. See [`super::view`] for why.
//!
//! # What this slice wires
//!
//! - The view dir (symlinks per promoted repo; a real harness config dir with
//!   `commands/` symlinked in from the hall; the active plan projected in; the
//!   bootstrap instructions — see [`super::view`]).
//! - The harness spawn through [`crate::harness`] with real `portable-pty`.
//! - The TUI loop, driven by the [`crate::tui`] modules.
//!
//! # Discovery sessions
//!
//! Naming no feature starts a **discovery session**: the view dir still holds
//! one symlink per repo, but every repo points at its read-only default-branch
//! worktree and the session record binds nothing. It lives under
//! `.ivar/sessions/<id>/` instead of a feature's tree, and `session convert` is
//! what later binds it to a feature — once, one way.
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
use crate::infra::progress::Progress;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::tui;
use crate::tui::driver::{Driver, PtsPty, Pty, ShellSpec};

use super::super::{discover_hall, read_manifest};
use super::{hook, lookup, view};

/// What `ivar session start` needs.
#[derive(Debug, Clone)]
pub struct StartInput {
    /// The feature to open a session for. `None` starts a **discovery
    /// session**: no feature bound, every repo read-only on its default
    /// branch.
    pub feature: Option<String>,
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
    /// The feature this session is bound to. `None` for a discovery session.
    pub feature: Option<FeatureName>,
    /// The provider that ran.
    pub provider: Provider,
    /// The session id (a UUID, from the view dir's name).
    pub session_id: String,
    /// Whether the session was created detached (no provider launched).
    pub detached: bool,
}

impl WriteHuman for StartOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let subject = match &self.feature {
            Some(feature) => format!("Session `{}` for feature `{feature}`", self.session_id),
            None => format!("Discovery session `{}`", self.session_id),
        };
        if self.detached {
            writeln!(
                w,
                "{subject} started detached (no provider launched). View dir: {}",
                self.view_dir
            )
        } else {
            writeln!(w, "{subject} ended. View dir: {}", self.view_dir)
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

    let feature = match input.feature {
        Some(raw) => Some(read_feature(&layout, FeatureName::new(raw)?)?),
        None => None,
    };

    // An unrestricted session cannot coexist with a successful partial
    // integration: a session with no write contract could move a locked
    // promotion. Refused before the smart fetch, the view dir, the hooks, or
    // any spawn.
    if let Some(feature) = &feature {
        crate::action::feature::ensure_unrestricted_session_allowed(&layout, feature)?;
    }

    let provider = resolve_provider(&manifest, input.provider.as_deref())?;
    let harness = Harness::for_provider(provider)?;
    if input.resume {
        harness::check_resume_supported(harness)?;
    }
    if input.relay {
        let feature = feature.as_ref().ok_or_else(relay_needs_feature)?;
        check_relay(&layout, &feature.name, input.provider.as_deref(), provider)?;
    }

    // 1. Smart Fetch, before the view dir exists: refresh every registered
    //    repo's default branch, best-effort per repo (valhalla's Smart Fetch).
    //    The refresh is fetch-and-fast-forward of the read-only default
    //    worktree — never a promoted repo's feature worktree — and a fetch
    //    that lands new files needs a writable target, so this runs before
    //    the guard-applying materialisation below.
    let mut warnings = smart_fetch(&git::System, &layout, &manifest, ctx.progress());

    // 2. The view dir and the session record. A discovery session lives in
    //    the hall's own session tree, and its record stays unbound.
    let session_id = SessionId::new(uuid::Uuid::new_v4().to_string())?;
    let view_dir = match &feature {
        Some(feature) => layout.feature_session(&feature.name, &session_id),
        None => layout.discovery_session(&session_id),
    };
    view::materialise(&layout, &manifest, feature.as_ref(), provider, &view_dir)?
        .warnings
        .into_iter()
        .for_each(|warning| warnings.push(warning));
    let started_at = rfc3339_now();
    let mut state = SessionState::new(provider, &started_at);
    if let Some(feature) = &feature {
        state.bind(feature.name.clone(), &started_at);
    }
    state.write(&view_dir)?;

    // 3. Session hooks, after the view dir and the record exist (a hook reads
    //    `IVAR_SESSION_PATH`) and before the agent spawns (a hook brings up the
    //    database the agent is about to talk to). Failure warns; it never
    //    stops the session — see `session::hook`. A discovery session
    //    promotes nothing, so there is no worktree for a hook to run in.
    if let Some(feature) = &feature {
        warnings.extend(hook::run_session_hooks(
            &layout,
            &manifest,
            feature,
            &view_dir,
            &session_id,
        ));
    }

    // 4. The agent command — skipped entirely for a detached session.
    if !input.detached {
        let command = harness.start_command(input.resume);
        let width = crate::infra::term::width();
        let height = 24;

        // If we are on a tty, run the TUI; otherwise spawn without it.
        if crate::infra::term::is_tty(crate::infra::term::Stream::Stdout) {
            run_tui(command, &view_dir, &layout, width, height)?;
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
            feature: feature.map(|feature| feature.name),
            provider,
            session_id: session_id.to_string(),
            detached: input.detached,
        },
        warnings,
    ))
}

/// Read the named feature, or refuse: a session cannot open over a feature
/// that was never created.
fn read_feature(layout: &Layout, name: FeatureName) -> Result<Feature, Failure> {
    Feature::read(layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })
}

/// A relay hands one feature's work to a different provider. With no feature
/// named there is no work to hand over.
fn relay_needs_feature() -> Failure {
    Failure::blocked(
        "session.relay_needs_feature",
        "a relay must name the feature to relay on",
    )
    .expected("a feature argument on a relay")
    .actual("no feature given, which would be a discovery session")
    .fix(FixAction::safe(
        "session.relay_pass_feature",
        "Pass the feature to relay on, or drop `--relay` to start a discovery session.",
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
///
/// `progress` is the same transient stderr line `repo pull` uses: this runs
/// before anything is printed and costs one network round trip per repo, which
/// is the longest a `session start` sits silent.
fn smart_fetch(
    git: &impl git::Git,
    layout: &Layout,
    manifest: &Manifest,
    progress: &dyn Progress,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let total = manifest.repos().len();
    for (index, repo) in manifest.repos().iter().enumerate() {
        progress.step(&pull::fetch_step(index, total, repo));
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
    progress.clear();
    warnings
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
    // Sized to the panel, not to the terminal: the agent draws inside the
    // right-hand box, so that is the width its lines have to wrap at.
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let (panel_width, panel_height) = tui::widget::panel_size(area);
    let mut driver = Driver::new(shells, PtsPty::new, panel_width, panel_height);

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
    let prefix = tui::master_detail::Prefix::from_env();
    let snapshot = driver.snapshot(layout.root().as_str(), &rows, prefix.label());
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
#[path = "../../../tests/unit/action/session/start.rs"]
mod tests;
