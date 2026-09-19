//! `ivar repo create` — seed a new repo and register it in the hall.
//!
//! Creates a README-only commit on the default branch, pushes to either the
//! hall origin (local) or a new GitHub repo (remote), then delegates the
//! bare clone, worktree, and manifest entry to [`repo::add`].

use std::fmt;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use super::add;
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::name::{BranchName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::proc;
use crate::store::manifest::local_ref_prefix;

// ── types ───────────────────────────────────────────────────────────

/// Whether to create the repo locally (in the hall origin) or on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CreateMode {
    /// Seed inside the hall origin under `repos/<name>/`.
    Local,
    /// Create a new GitHub repo (private by default, public when requested).
    Remote { public: bool },
}

impl fmt::Display for CreateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CreateMode::Local => write!(f, "local"),
            CreateMode::Remote { public: true } => write!(f, "public remote"),
            CreateMode::Remote { public: false } => write!(f, "private remote"),
        }
    }
}

/// What the caller wants to create.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The repo name (validated by `RepoName::new`).
    pub name: String,
    /// Local or remote.
    pub mode: CreateMode,
    /// Optional default branch override (defaults to `main`).
    pub default_branch: Option<String>,
}

/// What `ivar repo create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: camino::Utf8PathBuf,
    /// The repo name, as now recorded in `ivar.json`.
    pub repo: RepoName,
    /// The mode used.
    pub mode: CreateMode,
    /// The git remote URL.
    pub url: String,
    /// The ref prefix, if any.
    pub ref_prefix: Option<String>,
    /// Guided follow-up.
    pub next_action: String,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Created {} repo `{}` at {}",
            self.mode, self.repo, self.url
        )?;
        writeln!(w, "Next: run `{}`", self.next_action)
    }
}

// ── create ──────────────────────────────────────────────────────────

/// Create a new repo, seed it with a README, and register it in the hall.
///
/// The manifest is rewritten **after** the push lands, so a repo that fails
/// to seed never leaves a half-declared entry behind.
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let name = RepoName::new(input.name.clone())?;
    let default_branch = match input.default_branch.as_deref() {
        Some(raw) => BranchName::new(raw)?,
        None => BranchName::new("main").map_err(|_| {
            Failure::blocked(
                "repo.create_needs_branch",
                "cannot default to `main`: `main` is not a valid branch name",
            )
            .expected("a valid default branch")
            .actual("`main` was refused by git's branch-name rules")
            .fix(FixAction::safe(
                "repo.pass_branch",
                "Pass --default-branch with a branch name git accepts.",
            ))
        })?,
    };

    // Collision: the name must be free.
    super::ensure_name_free(&manifest, &name)?;

    let (url, ref_prefix) = match &input.mode {
        CreateMode::Local => {
            let url = hall_origin_url(layout.root())?;
            let prefix = local_ref_prefix(&name);
            (url, Some(prefix))
        }
        CreateMode::Remote { public } => remote::provision(&name, *public)?,
    };

    seed_and_push(&url, default_branch.as_str(), &name, ref_prefix.as_deref())?;

    let add_input = add::AddInput {
        name: input.name.clone(),
        url: url.clone(),
        default_branch: Some(default_branch.as_str().to_owned()),
        reuse_existing: None,
        ref_prefix: ref_prefix.clone(),
    };
    let add_outcome = add::add(ctx, add_input)?;

    Ok(Report::new(CreateOutcome {
        root: add_outcome.value.root,
        repo: name,
        mode: input.mode,
        url,
        ref_prefix,
        next_action: add_outcome.value.next_action,
    }))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Get the origin URL of the hall's own git repository.
fn hall_origin_url(root: &Utf8Path) -> Result<String, Failure> {
    let output = proc::capture(
        &proc::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .cwd(root),
    )
    .map_err(|e| Failure::failed("repo.hall_has_no_origin", e.to_string()))?;

    if output.success() {
        Ok(output.stdout.trim().to_owned())
    } else {
        Err(
            Failure::blocked("repo.hall_has_no_origin", "hall has no `origin` remote").fix(
                FixAction::safe(
                    "repo.set_origin",
                    format!("In `{root}`, run `git remote add origin <url>`."),
                ),
            ),
        )
    }
}

/// Seed a temporary git repo with a README commit and push it to `url`.
fn seed_and_push(
    url: &str,
    branch: &str,
    name: &RepoName,
    ref_prefix: Option<&str>,
) -> Result<(), Failure> {
    let dir = tempfile::TempDir::new()
        .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;
    let dir_path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).map_err(|p| {
        Failure::failed(
            "repo.seed_push_failed",
            format!("path is not UTF-8: {}", p.display()),
        )
    })?;
    // git init
    let output = proc::capture(
        &proc::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg("-b")
            .arg(branch)
            .cwd(&dir_path),
    )
    .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;
    if !output.success() {
        return Err(Failure::failed(
            "repo.seed_push_failed",
            output.diagnostic(),
        ));
    }

    // Write README
    std::fs::write(dir_path.join("README.md"), format!("# {name}\n"))
        .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;

    // git add
    let output = proc::capture(
        &proc::Command::new("git")
            .arg("add")
            .arg("README.md")
            .cwd(&dir_path),
    )
    .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;
    if !output.success() {
        return Err(Failure::failed(
            "repo.seed_push_failed",
            output.diagnostic(),
        ));
    }

    // git commit
    let output = proc::capture(
        &proc::Command::new("git")
            .args(["-c", "user.name=ivar"])
            .args(["-c", "user.email=ivar@localhost"])
            .args(["commit", "-q", "-m", "chore: initial commit"])
            .cwd(&dir_path),
    )
    .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;
    if !output.success() {
        return Err(Failure::failed(
            "repo.seed_push_failed",
            output.diagnostic(),
        ));
    }

    // git push
    let output = proc::capture(
        &proc::Command::new("git")
            .arg("push")
            .arg("-q")
            .arg(url)
            .arg(match ref_prefix {
                Some(prefix) => format!("{branch}:refs/heads/{prefix}{branch}"),
                None => format!("{branch}:refs/heads/{branch}"),
            })
            .cwd(&dir_path),
    )
    .map_err(|e| Failure::failed("repo.seed_push_failed", e.to_string()))?;
    if !output.success() {
        return Err(Failure::failed(
            "repo.seed_push_failed",
            output.diagnostic(),
        ));
    }

    Ok(())
}

// ── remote ──────────────────────────────────────────────────────────

/// Remote provisioning helpers.
mod remote {
    use crate::domain::name::RepoName;
    use crate::error::{Failure, FixAction};
    use crate::infra;

    /// Create or validate a GitHub repo, returning `(url, None)`.
    pub(super) fn provision(
        name: &RepoName,
        public: bool,
    ) -> Result<(String, Option<String>), Failure> {
        let login = infra::github::gh_login()?;
        let full_name = format!("{login}/{name}");

        if infra::github::gh_repo_exists(&full_name)? {
            return Err(Failure::blocked(
                "repo.github_repo_exists",
                format!("`{full_name}` already exists on GitHub"),
            )
            .expected("a repo name not already on GitHub")
            .actual(format!("`{full_name}` already exists"))
            .fix(FixAction::safe(
                "repo.use_add",
                format!("Run `ivar repo add {name} https://github.com/{full_name}` instead."),
            )));
        }

        let url = infra::github::gh_repo_create(&full_name, public)?;
        Ok((url, None))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/repo/create.rs"]
mod tests;
