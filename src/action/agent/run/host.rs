//! The `runtime: host` run: the tool on the host, confined by Landlock.

use camino::{Utf8Path, Utf8PathBuf};

use super::{Ran, Workspace};
use crate::action::session::guard::canonicalize_lenient;
use crate::domain::agent::host_env::host_env;
use crate::domain::agent::spec::{AgentSpec, Launch, WritePath};
use crate::domain::agent::tool::Credential;
use crate::error::Failure;
use crate::infra::fs;
use crate::infra::proc::{self, Command};
use crate::store::layout::Layout;

#[derive(Debug, Clone)]
pub(crate) struct HostDeps {
    pub launcher: Utf8PathBuf,
    pub home: Utf8PathBuf,
    pub parent_env: Vec<(String, String)>,
    pub landlock: bool,
}

impl HostDeps {
    pub(crate) fn system() -> Result<Self, Failure> {
        let parent_env: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        let home = parent_env
            .iter()
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| Utf8PathBuf::from(v))
            .ok_or_else(|| {
                Failure::failed(
                    "agent.home_unknown",
                    "HOME is not set; a `runtime: host` run uses the tool's login under it",
                )
            })?;
        Ok(Self {
            launcher: super::super::sync::current_ivar_bin()?,
            home,
            parent_env,
            landlock: crate::action::session::sandbox::landlock_available(),
        })
    }
}

/// The Docker `Workspace` on the host: `scratch/workspace/<repo>` links to each
/// mounted worktree; only `read_only: false` mounts become writable roots.
pub(crate) fn host_workspace(
    scratch: &Utf8Path,
    ws: &Workspace,
) -> Result<(Utf8PathBuf, Vec<Utf8PathBuf>), Failure> {
    let names = ws
        .mounts
        .iter()
        .map(|mount| {
            mount
                .target
                .strip_prefix("/workspace/")
                .filter(|n| !n.is_empty() && !n.contains('/') && *n != ".." && *n != ".")
                .ok_or_else(|| {
                    Failure::failed(
                        "agent.invalid_mount",
                        format!(
                            "mount target `{}` is not a `/workspace/<name>` path",
                            mount.target
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = scratch.join("workspace");
    fs::ensure_dir(&root)?;
    for (mount, name) in ws.mounts.iter().zip(names) {
        fs::create_symlink(Utf8Path::new(&mount.source), &root.join(name))?;
    }
    let cwd = root.join(
        ws.workdir
            .trim_start_matches("/workspace")
            .trim_start_matches('/'),
    );
    let writable = ws
        .mounts
        .iter()
        .filter(|m| !m.read_only)
        .map(|m| Utf8PathBuf::from(&m.source))
        .collect();
    Ok((cwd, writable))
}

/// A login file as it was copied into the state dir.
pub(crate) struct Seeded {
    credential: Credential,
    copied: Option<Vec<u8>>,
}

/// Copies each login file into `state` (0700), replacing a stale copy and
/// dropping one whose source is gone.
pub(crate) fn seed_credentials(
    state: &Utf8Path,
    credentials: Vec<Credential>,
) -> Result<Vec<Seeded>, Failure> {
    fs::ensure_dir(state)?;
    #[cfg(unix)]
    fs::chmod(state, 0o700)?;
    credentials
        .into_iter()
        .map(|credential| {
            let private = state.join(credential.private);
            let copied = fs::read_bytes(&credential.real)?;
            match &copied {
                Some(bytes) => {
                    if let Some(parent) = private.parent() {
                        fs::ensure_dir(parent)?;
                    }
                    fs::write_sensitive_atomic(&private, bytes)?;
                }
                None => fs::remove_path(&private)?,
            }
            Ok(Seeded { credential, copied })
        })
        .collect()
}

fn is_login(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.as_object().map(|o| !o.is_empty()))
        .unwrap_or(false)
}

/// Writes a token the tool refreshed during the run back to the user's own
/// login, so a rotated refresh token does not log them out. Only a valid
/// login is written, and never over one the user changed meanwhile.
pub(crate) fn write_back_credentials(state: &Utf8Path, seeded: &[Seeded]) -> Result<(), Failure> {
    for Seeded { credential, copied } in seeded.iter().filter(|s| s.credential.write_back) {
        let Some(copied) = copied else { continue };
        let Some(now) = fs::read_bytes(&state.join(credential.private))? else {
            continue;
        };
        if &now == copied || !is_login(&now) {
            continue;
        }
        if fs::read_bytes(&credential.real)?.as_ref() != Some(copied) {
            continue;
        }
        fs::write_sensitive_atomic(&credential.real, &now)?;
    }
    Ok(())
}

fn protect(raw: &Utf8Path) -> Result<Utf8PathBuf, Failure> {
    let canonical = canonicalize_lenient(raw);
    if canonical.as_str().is_empty() {
        return Err(Failure::failed(
            "agent.write_refused",
            format!(
                "protected path `{raw}` does not resolve (a symlink loop or a dangling link through `..`); fix it before a run can use `write`"
            ),
        ));
    }
    Ok(canonical)
}

/// Canonical Landlock roots for `write`. Landlock grants a whole subtree, so a
/// root may not contain the home, the hall, the tool's login or a path the
/// session write guard protects, nor sit inside one of those protected paths,
/// the hall's state, git or agent files, or the tool's login directory. A login
/// directory that holds the home or the hall refuses every `write`.
pub(crate) fn write_roots(
    writes: &[WritePath],
    home: &Utf8Path,
    hall: &Utf8Path,
    credentials: &[Credential],
) -> Result<Vec<Utf8PathBuf>, Failure> {
    if writes.is_empty() {
        return Ok(Vec::new());
    }
    let (home, hall) = (canonicalize_lenient(home), canonicalize_lenient(hall));
    let guarded = Layout::at(hall.clone())
        .guard_protected_paths()
        .iter()
        .map(|p| protect(p))
        .collect::<Result<Vec<_>, _>>()?;
    let login_dirs = credentials
        .iter()
        .filter(|c| c.write_back)
        .filter_map(|c| c.real.parent())
        .map(protect)
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(dir) = login_dirs
        .iter()
        .find(|dir| home.starts_with(dir) || hall.starts_with(dir))
    {
        return Err(Failure::failed(
            "agent.write_refused",
            format!(
                "the tool's login directory `{dir}` holds the home or the hall, so its config cannot be kept out of `write`; point CLAUDE_CONFIG_DIR, CODEX_HOME or XDG_DATA_HOME at a dedicated directory"
            ),
        ));
    }
    let logins = credentials
        .iter()
        .map(|c| protect(&c.real))
        .collect::<Result<Vec<_>, _>>()?;
    let enclosing: Vec<Utf8PathBuf> = [".ivar", ".git", "agents"]
        .iter()
        .map(|dir| hall.join(dir))
        .chain(login_dirs)
        .chain(guarded.iter().cloned())
        .collect();
    let contained: Vec<Utf8PathBuf> = [home.clone(), hall.clone()]
        .into_iter()
        .chain(logins)
        .chain(guarded)
        .collect();
    writes
        .iter()
        .map(|write| {
            let path = write.resolve(&home, &hall);
            let root = fs::canonicalize(&path).map_err(|_| {
                Failure::failed(
                    "agent.write_missing",
                    format!("`write` path `{path}` does not exist; create it before the run"),
                )
            })?;
            let collision = contained
                .iter()
                .find(|p| p.starts_with(&root))
                .or_else(|| enclosing.iter().find(|p| root.starts_with(p)));
            if let Some(protected) = collision {
                return Err(Failure::failed(
                    "agent.write_refused",
                    format!(
                        "`write` path `{path}` resolves to `{root}`, which would make `{protected}` writable"
                    ),
                ));
            }
            Ok(root)
        })
        .collect()
}

pub(super) fn execute_host(
    spec: &AgentSpec,
    hall: &Utf8Path,
    scratch: &Utf8Path,
    state: &Utf8Path,
    secrets: &[(String, String)],
    ws: &Workspace,
    host: &HostDeps,
) -> Result<Ran, Failure> {
    if !host.landlock {
        return Err(Failure::blocked(
            "agent.landlock_unavailable",
            "Landlock is unavailable on this kernel, so a `runtime: host` run cannot be confined; use `runtime: docker` or Linux 5.13+",
        ));
    }
    let Launch::Tool { tool, model } = &spec.launch else {
        return Err(Failure::failed(
            "agent.invalid",
            "`runtime: host` needs `tool`",
        ));
    };
    let var = |key: &str| {
        host.parent_env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let credentials = tool.credentials(&host.home, &var);
    let extra = write_roots(&spec.write, &host.home, hall, &credentials)?;
    let (cwd, writable) = host_workspace(scratch, ws)?;
    let tmp = scratch.join("tmp");
    fs::ensure_dir(&tmp)?;
    let seeded = seed_credentials(state, credentials)?;
    let mut roots = vec![
        scratch.to_path_buf(),
        state.to_path_buf(),
        Utf8PathBuf::from("/dev/null"),
    ];
    roots.extend(writable);
    roots.extend(extra);
    let mut command = Command::new(host.launcher.as_str()).args(["agent", "sandbox"]);
    for root in &roots {
        command = command.arg("--write").arg(root.as_str());
    }
    command = command
        .arg("--")
        .args(tool.argv(model.as_deref(), &spec.prompt))
        .cwd(cwd)
        .env_clear()
        .process_group();
    for (key, value) in host_env(
        &host.parent_env,
        tmp.as_str(),
        &spec.env,
        secrets,
        *tool,
        state,
    ) {
        command = command.env(key, value);
    }
    let log = scratch.join("output.log");
    let finished =
        proc::to_log_with_deadline(&command, &log, spec.timeout, &|seen| tool.is_blocked(seen));
    let written_back = write_back_credentials(state, &seeded);
    let finished = finished?;
    written_back?;
    Ok(Ran {
        finished,
        output: fs::read_text(&log)?.unwrap_or_default(),
    })
}
