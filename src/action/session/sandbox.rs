//! Kernel-enforced write sandbox: computes the footprint of allowed write roots
//! from a `WritableSet`, backing git repositories, platform devices, and provider
//! runtime state.

use crate::action::session::guard::{WritableSet, canonicalize_lenient};
use crate::domain::feature::Feature;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// Status of the kernel-enforced write sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SandboxStatus {
    /// Ruleset is fully enforced by the kernel.
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "only constructed by the Linux Landlock path; the test build does construct it"
        )
    )]
    Enforced,
    /// Ruleset is partially enforced (e.g. kernel supports an older Landlock ABI).
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "only constructed by the Linux Landlock path; the test build does construct it"
        )
    )]
    Degraded { reason: String },
    /// Landlock is unavailable on this kernel or platform (e.g. macOS or Linux < 5.13).
    Unavailable { reason: String },
}

impl SandboxStatus {
    /// Returns true if the sandbox is actively and fully enforcing kernel write restrictions.
    #[cfg(test)]
    pub(crate) fn is_enforced(&self) -> bool {
        matches!(self, Self::Enforced)
    }
}

/// The kernel-enforced write sandbox holding the derived set of write-allowed filesystem roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sandbox {
    roots: Vec<Utf8PathBuf>,
    temp_root: Utf8PathBuf,
}

impl Sandbox {
    /// Derive the complete list of write-allowed filesystem roots for a session.
    pub(crate) fn from_writable_set(
        set: &WritableSet,
        layout: &Layout,
        feature: Option<&Feature>,
        provider: Provider,
        session: &SessionId,
    ) -> Result<Self, Failure> {
        // Ensure canonical hall source directories exist before filtering nonexistent paths.
        crate::infra::fs::ensure_dir(&layout.hall_skills())?;
        crate::infra::fs::ensure_dir(&layout.hall_skills_local())?;
        crate::infra::fs::ensure_dir(&layout.hall_setups())?;

        let mut candidate_roots: Vec<Utf8PathBuf> = Vec::new();

        // 1. Primary write roots from the WritableSet.
        candidate_roots.extend(set.roots()?);

        // 2. Feature directory grants for feature sessions:
        // For a feature session, grant .ivar/features/ and .ivar/repos/<repo>/ for each promoted repo.
        // This enables mid-session creation of child features, detached child sessions, and child worktrees.
        if let Some(feature) = feature {
            candidate_roots.push(layout.features_dir());
            for repo in feature.promotions.keys() {
                candidate_roots.push(layout.repo_dir(repo));
                candidate_roots.push(layout.repo_bare(repo));
            }
        }

        // 3. /dev/null - required by git and various tools for write redirection.
        let dev_null = Utf8PathBuf::from("/dev/null");
        candidate_roots.push(dev_null);

        // 4. Temporary directory for compiler/tool outputs.
        let temp_dir = Utf8PathBuf::try_from(std::env::temp_dir())
            .unwrap_or_else(|_| Utf8PathBuf::from("/tmp"));
        let temp_dir = temp_dir.canonicalize_utf8().unwrap_or(temp_dir);
        let hall_paths = hall_paths(layout);
        let temp_root = sandbox_temp_root(&temp_dir, &hall_paths, session);
        if temp_root != temp_dir {
            create_private_dir(&temp_root, &canonicalize_lenient(layout.root()))?;
        }
        candidate_roots.push(temp_root.clone());

        // 5. Shared cargo target cache if it exists under the hall layout.
        let cache_dir = layout.root().join(".ivar").join("cache");
        candidate_roots.push(cache_dir);

        // 6. Provider runtime directories.
        for p_root in provider_runtime_roots(provider) {
            candidate_roots.push(p_root);
        }

        // Canonicalise and filter candidate roots to only those that exist on disk.
        // Landlock PathFd::new fails if a path does not exist.
        let mut final_roots: Vec<Utf8PathBuf> = Vec::new();
        for path in candidate_roots {
            if let Ok(canonical) = path.canonicalize_utf8() {
                if !final_roots.contains(&canonical) {
                    final_roots.push(canonical);
                }
            } else if path.exists() && !final_roots.contains(&path) {
                final_roots.push(path);
            }
        }

        Ok(Self {
            roots: final_roots,
            temp_root,
        })
    }

    /// The one temp dir the session may write, exported to the provider as `TMPDIR`.
    pub(crate) fn temp_root(&self) -> &Utf8Path {
        &self.temp_root
    }

    /// Return the list of canonical roots that will be added to the ruleset.
    #[cfg(test)]
    pub(crate) fn roots(&self) -> &[Utf8PathBuf] {
        &self.roots
    }

    /// Apply the write-only ruleset to the calling process.
    ///
    /// On Linux, builds a Landlock ruleset covering all `self.roots()`, handles only
    /// write-class filesystem access (keeping reads and execution ambient), and restricts
    /// the calling process with `no_new_privs`.
    ///
    /// On non-Linux platforms, returns `SandboxStatus::Unavailable` without failing.
    #[cfg(target_os = "linux")]
    pub(crate) fn apply(&self) -> Result<SandboxStatus, Failure> {
        use landlock::{
            ABI, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
            RulesetStatus, Scope,
        };

        let abi = ABI::V5;
        // WRITE-only: reads and execs stay ambient (R-READ).
        let write_rights = AccessFs::from_write(abi);

        let mut ruleset = match Ruleset::default()
            .handle_access(write_rights)
            .and_then(|r| r.scope(Scope::Signal | Scope::AbstractUnixSocket))
            .and_then(|r| r.create())
        {
            Ok(ruleset) => ruleset,
            Err(err) => {
                let err_str = err.to_string();
                if err_str.contains("unsupported")
                    || err_str.contains("ENOSYS")
                    || err_str.contains("Function not implemented")
                    || err_str.contains("Operation not supported")
                {
                    return Ok(SandboxStatus::Unavailable {
                        reason: format!("Landlock is not supported by this kernel: {err}"),
                    });
                }
                return Err(Failure::failed(
                    "sandbox.ruleset_create_failed",
                    format!("failed to initialize Landlock ruleset: {err}"),
                ));
            }
        };

        for path in &self.roots {
            let path_fd = match PathFd::new(path.as_std_path()) {
                Ok(fd) => fd,
                Err(err) => {
                    let err_str = err.to_string();
                    if err_str.contains("No such file") || err_str.contains("not found") {
                        continue;
                    }
                    return Err(Failure::failed(
                        "sandbox.path_fd_failed",
                        format!("failed to open path descriptor for `{path}`: {err}"),
                    ));
                }
            };

            // A file rule (like /dev/null or a config file) accepts only file rights; any
            // directory right on it (Refer, MakeDir, ...) is stripped by the crate, which
            // then reports the whole ruleset as partially enforced.
            let rights = if path.is_dir() {
                write_rights
            } else {
                write_rights & (AccessFs::WriteFile | AccessFs::Truncate)
            };

            ruleset = match ruleset.add_rule(PathBeneath::new(path_fd, rights)) {
                Ok(r) => r,
                Err(err) => {
                    return Err(Failure::failed(
                        "sandbox.add_rule_failed",
                        format!("failed to add sandbox write rule for `{path}`: {err}"),
                    ));
                }
            };
        }

        let restriction = match ruleset.restrict_self() {
            Ok(r) => r,
            Err(err) => {
                let err_str = err.to_string();
                if err_str.contains("unsupported")
                    || err_str.contains("ENOSYS")
                    || err_str.contains("Operation not supported")
                {
                    return Ok(SandboxStatus::Unavailable {
                        reason: format!("Landlock self-restriction unsupported: {err}"),
                    });
                }
                return Err(Failure::failed(
                    "sandbox.restrict_self_failed",
                    format!("failed to restrict self with Landlock ruleset: {err}"),
                ));
            }
        };

        match restriction.ruleset {
            RulesetStatus::FullyEnforced => Ok(SandboxStatus::Enforced),
            RulesetStatus::PartiallyEnforced => Ok(SandboxStatus::Degraded {
                reason: "Landlock ruleset is partially enforced by the kernel".to_owned(),
            }),
            RulesetStatus::NotEnforced => Ok(SandboxStatus::Unavailable {
                reason: "Landlock ruleset was not enforced by the kernel".to_owned(),
            }),
        }
    }

    /// Apply fallback for non-Linux platforms where Landlock is unavailable.
    #[cfg(not(target_os = "linux"))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "must match the fallible Linux apply signature"
    )]
    pub(crate) fn apply(&self) -> Result<SandboxStatus, Failure> {
        Ok(SandboxStatus::Unavailable {
            reason: format!("Landlock is not supported on {}", std::env::consts::OS),
        })
    }
}

/// Derive candidate runtime and state directories for the given provider.
fn provider_runtime_roots(provider: Provider) -> Vec<Utf8PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").ok().map(Utf8PathBuf::from);

    match provider {
        Provider::ClaudeCode => {
            if let Some(home) = &home {
                dirs.push(home.join(".claude"));
                dirs.push(home.join(".claude.json"));
            }
        }
        Provider::Omp => {
            if let Some(home) = &home {
                dirs.push(home.join(".omp"));
            }
            if let Ok(pi_config) = std::env::var("PI_CONFIG_DIR") {
                dirs.push(Utf8PathBuf::from(pi_config));
            }
        }
        Provider::OpenCode => {
            if let Some(home) = &home {
                dirs.push(home.join(".local").join("share").join("opencode"));
                dirs.push(home.join(".local").join("state").join("opencode"));
                dirs.push(home.join(".config").join("opencode"));
            }
            if let Ok(data_dir) = crate::infra::fs::data_dir() {
                dirs.push(data_dir.join("opencode"));
            }
            if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
                dirs.push(Utf8PathBuf::from(xdg_state).join("opencode"));
            }
            if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
                dirs.push(Utf8PathBuf::from(xdg_config).join("opencode"));
            }
        }
    }
    dirs
}

/// The hall root, `.ivar` and the protected paths, each resolved the
/// way the guard resolves a target: a protected path that is a dangling
/// symlink lives where its target would be created.
fn hall_paths(layout: &Layout) -> Vec<Utf8PathBuf> {
    [layout.root().to_path_buf(), layout.ivar_dir()]
        .into_iter()
        .chain(layout.guard_protected_paths())
        .map(|path| canonicalize_lenient(&path))
        .collect()
}

/// The temp dir a session may write. `hall_paths` comes from [`hall_paths`]. A temp dir that contains any of them, or lies
/// inside one, would grant it, so it narrows to a private per-session dir
/// beneath it.
pub(crate) fn sandbox_temp_root(
    temp_dir: &Utf8Path,
    hall_paths: &[Utf8PathBuf],
    session: &SessionId,
) -> Utf8PathBuf {
    if hall_paths
        .iter()
        .any(|path| path.starts_with(temp_dir) || temp_dir.starts_with(path))
    {
        temp_dir.join(format!("ivar-{session}"))
    } else {
        temp_dir.to_path_buf()
    }
}

/// Create the private temp dir, or accept an existing one only when it is a
/// real directory, private, and owned by the hall's owner: the dir sits in a
/// shared temp dir under a predictable name, and a planted symlink would
/// redirect the kernel grant wherever it points.
#[cfg(unix)]
fn create_private_dir(dir: &Utf8Path, hall_root: &Utf8Path) -> Result<(), Failure> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(Failure::failed(
                "sandbox.temp_root_failed",
                format!("could not create the session temp dir `{dir}`: {err}"),
            ));
        }
    }
    let trusted = match (std::fs::symlink_metadata(dir), std::fs::metadata(hall_root)) {
        (Ok(meta), Ok(hall)) => {
            meta.file_type().is_dir()
                && meta.uid() == hall.uid()
                && meta.permissions().mode() & 0o077 == 0
        }
        _ => false,
    };
    if trusted {
        return Ok(());
    }
    Err(Failure::blocked(
        "sandbox.untrusted_temp_root",
        format!(
            "the session temp dir `{dir}` exists but is not a private directory owned by the hall's owner"
        ),
    )
    .fix(crate::error::FixAction::safe(
        "sandbox.remove_untrusted_temp_root",
        format!("Inspect and remove `{dir}`, then relaunch the session."),
    )))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Utf8Path, _hall_root: &Utf8Path) -> Result<(), Failure> {
    crate::infra::fs::ensure_dir(dir)?;
    Ok(())
}

/// Run the internal launcher: resolve the session from disk, rebuild its
/// provider launch, apply the sandbox, and exec the provider.
///
/// The launch is built before the ruleset is applied: building it reads the
/// manifest and the secret store, and there is no reason to do either under
/// a write restriction.
#[allow(clippy::print_stderr)]
pub fn run_launcher(
    ctx: &crate::action::Ctx,
    session_id_str: &str,
    resume: bool,
    argv: &[String],
) -> Result<(), Failure> {
    let Some((program, user_args)) = argv.split_first() else {
        return Err(Failure::blocked(
            "sandbox.launcher_missing_command",
            "no command specified to run inside sandbox",
        ));
    };

    let layout = crate::action::discover_hall(ctx)?;
    let manifest = crate::action::read_manifest(&layout)?;

    let session_ref = crate::action::session::lookup::resolve(&layout, Some(session_id_str), None)?;
    let state = session_ref.state.as_ref().ok_or_else(|| {
        Failure::blocked(
            "session.state_missing",
            format!("session `{session_id_str}` has no state.json record"),
        )
    })?;
    super::launch::ensure_provider_binary(state.provider, program)?;

    let feature = match &session_ref.feature {
        Some(feat_name) => Feature::read(&layout, feat_name)?,
        None => None,
    };

    let command = super::launch::provider_command(
        &layout,
        &manifest,
        state.provider,
        &session_ref.view_dir,
        &session_ref.id,
        session_ref.feature.as_ref(),
        resume,
        user_args,
    )?;

    let set = match &feature {
        Some(feat) => WritableSet::from_session(&layout, feat, &session_ref.view_dir)?,
        None => WritableSet::from_discovery(&layout, &session_ref.view_dir)?,
    };

    let sandbox = Sandbox::from_writable_set(
        &set,
        &layout,
        feature.as_ref(),
        state.provider,
        &session_ref.id,
    )?;
    let command = command.env("TMPDIR", sandbox.temp_root().as_str());
    let status = sandbox.apply()?;

    match &status {
        SandboxStatus::Enforced => {
            eprintln!("[ivar] write guard: kernel Landlock sandbox active");
        }
        SandboxStatus::Degraded { reason } => {
            eprintln!("[ivar] write guard: degraded ({reason})");
        }
        SandboxStatus::Unavailable { reason } => {
            eprintln!("[ivar] write guard: unavailable ({reason})");
        }
    }

    match crate::infra::proc::exec(&command)? {
        Some(0) | None => Ok(()),
        Some(code) => Err(Failure::failed(
            "sandbox.process_failed",
            format!("`{program}` exited with {code}"),
        )
        .expected("the provider process to exit successfully with 0")
        .actual(format!("exit code {code}"))
        .fix(crate::error::FixAction::safe(
            "session.inspect_provider_output",
            "Inspect the provider output printed above for errors or diagnostics.",
        ))),
    }
}
#[cfg(test)]
#[path = "../../../tests/unit/action/session/sandbox.rs"]
mod tests;
