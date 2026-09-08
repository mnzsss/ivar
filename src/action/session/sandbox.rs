//! Kernel-enforced write sandbox: computes the footprint of allowed write roots
//! from a `WritableSet`, backing git repositories, platform devices, and provider
//! runtime state.

use crate::action::session::guard::WritableSet;
use crate::domain::feature::Feature;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::Utf8PathBuf;

/// Status of the kernel-enforced write sandbox.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SandboxStatus {
    /// Ruleset is fully enforced by the kernel.
    Enforced,
    /// Ruleset is partially enforced (e.g. kernel supports an older Landlock ABI).
    Degraded { reason: String },
    /// Landlock is unavailable on this kernel or platform (e.g. macOS or Linux < 5.13).
    Unavailable { reason: String },
}

impl SandboxStatus {
    /// Returns true if the sandbox is actively and fully enforcing kernel write restrictions.
    #[allow(dead_code)]
    pub(crate) fn is_enforced(&self) -> bool {
        matches!(self, Self::Enforced)
    }
}

/// The kernel-enforced write sandbox holding the derived set of write-allowed filesystem roots.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sandbox {
    roots: Vec<Utf8PathBuf>,
}

impl Sandbox {
    /// Derive the complete list of write-allowed filesystem roots for a session.
    #[allow(dead_code)]
    pub(crate) fn from_writable_set(
        set: &WritableSet,
        layout: &Layout,
        feature: Option<&Feature>,
        provider: Provider,
    ) -> Result<Self, Failure> {
        let mut candidate_roots: Vec<Utf8PathBuf> = Vec::new();

        // 1. Primary write roots from the WritableSet (view dir, feature dir, promoted worktrees).
        for root in set.roots() {
            candidate_roots.push(root.to_path_buf());
        }

        // 2. Backing git bare repos for any promoted repositories in the feature.
        // Required for git operations (index.lock, refs, objects) within worktrees.
        if let Some(feature) = feature {
            for repo in feature.promotions.keys() {
                candidate_roots.push(layout.repo_bare(repo));
            }
        }

        // 3. /dev/null - required by git and various tools for write redirection.
        let dev_null = Utf8PathBuf::from("/dev/null");
        candidate_roots.push(dev_null);

        // 4. Temporary directory for compiler/tool outputs.
        let temp_dir = Utf8PathBuf::try_from(std::env::temp_dir())
            .unwrap_or_else(|_| Utf8PathBuf::from("/tmp"));
        candidate_roots.push(temp_dir);

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

        Ok(Self { roots: final_roots })
    }

    /// Return the list of canonical roots that will be added to the ruleset.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

            // Non-directory file descriptors (like /dev/null or config files) cannot receive
            // directory-specific write rights (MakeDir, RemoveDir) in Landlock.
            let rights = if path.is_dir() {
                write_rights
            } else {
                write_rights & (AccessFs::WriteFile | AccessFs::Truncate | AccessFs::Refer)
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
    #[allow(dead_code)]
    #[cfg(not(target_os = "linux"))]
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

#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;

/// Run the internal launcher: resolve session by id from disk, apply sandbox, and exec target command.
#[allow(clippy::print_stderr)]
pub fn run_launcher(session_id_str: &str, argv: &[String]) -> Result<(), Failure> {
    if argv.is_empty() {
        return Err(Failure::blocked(
            "sandbox.launcher_missing_command",
            "no command specified to run inside sandbox",
        ));
    }
    let cwd = Utf8PathBuf::try_from(std::env::current_dir().map_err(|e| {
        Failure::failed("fs.current_dir_failed", format!("could not get cwd: {e}"))
    })?)
    .map_err(|e| Failure::failed("fs.utf8_error", format!("non-UTF8 cwd: {e}")))?;

    let layout = Layout::discover(&cwd)?.ok_or_else(|| {
        Failure::blocked("hall.not_found", "no hall found from current directory")
    })?;

    let session_ref = crate::action::session::lookup::resolve(&layout, Some(session_id_str), None)?;
    let state = session_ref.state.as_ref().ok_or_else(|| {
        Failure::blocked(
            "session.state_missing",
            format!("session `{session_id_str}` has no state.json record"),
        )
    })?;

    let feature = match &session_ref.feature {
        Some(feat_name) => Feature::read(&layout, feat_name)?,
        None => None,
    };

    let set = match &feature {
        Some(feat) => WritableSet::from_session(&layout, feat, &session_ref.view_dir)?,
        None => WritableSet::from_discovery(&session_ref.view_dir)?,
    };

    let sandbox = Sandbox::from_writable_set(&set, &layout, feature.as_ref(), state.provider)?;
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

    #[cfg(target_os = "linux")]
    {
        let prog = argv.first().ok_or_else(|| {
            Failure::blocked("sandbox.missing_argv", "no program specified to execute")
        })?;
        let mut cmd = std::process::Command::new(prog);
        if let Some(args) = argv.get(1..) {
            cmd.args(args);
        }
        let err = cmd.exec();
        Err(Failure::failed(
            "sandbox.exec_failed",
            format!("failed to exec `{prog}`: {err}"),
        ))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let prog = argv.first().ok_or_else(|| {
            Failure::blocked("sandbox.missing_argv", "no program specified to execute")
        })?;
        let mut cmd = std::process::Command::new(prog);
        if let Some(args) = argv.get(1..) {
            cmd.args(args);
        }
        let mut child = cmd.spawn().map_err(|e| {
            Failure::failed(
                "sandbox.spawn_failed",
                format!("failed to spawn `{prog}`: {e}"),
            )
        })?;
        let status = child.wait().map_err(|e| {
            Failure::failed(
                "sandbox.wait_failed",
                format!("failed to wait on `{prog}`: {e}"),
            )
        })?;
        if !status.success() {
            return Err(Failure::failed(
                "sandbox.process_failed",
                format!("process `{prog}` exited with {status}"),
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "../../../tests/unit/action/session/sandbox.rs"]
mod tests;
