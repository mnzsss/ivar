//! `hall-sync-resolver`: an agent shipped inside the binary that settles the
//! rebase conflicts the fixed rules of [`super::resolve`] do not cover.
//!
//! Two machines writing memory pages and docs conflict now and then, and a sync
//! that gives up at the first one stays stuck until a person looks. Before the
//! sync abandons the rebase it hands the conflicted files to a model, then
//! checks what came back before it lets git commit it.
//!
//! # How it runs
//!
//! Through [`run_spec`](crate::action::agent::run::run_spec), the structure
//! behind `ivar agent run`, with the hall's default provider as the tool. It is
//! a `runtime: host` run, so Landlock confines its writes to the conflicted
//! files and nothing else. It is an [`AgentSpec`] built here, not a file in the
//! hall's `agents/`: a hall cannot change its prompt or its limits, and it is
//! never scheduled.
//!
//! The run does not go through [`run`](crate::action::agent::run::run), the
//! entry point of the command. Anything that command does around a run, such as
//! syncing the hall afterwards, therefore cannot happen to this one: a sync
//! that runs the resolver can never start another sync.
//!
//! # What the agent is given
//!
//! Only the conflicted files that the fixed rules left: for each, the remote's
//! version, the local one, and the commit each came from. Its rule is to keep
//! the content of both sides, and never to drop one side of a prose file.
//!
//! # What is checked before the commit
//!
//! None of this trusts the agent:
//!
//! - the run succeeded, within its timeout;
//! - nothing else in the hall changed — not a file, not a byte of one that was
//!   already modified;
//! - no conflict marker (`<<<<<<<`, `=======`, `>>>>>>>`) is left in a
//!   resolved file;
//! - every page under `docs/memory/` still parses, frontmatter included.
//!
//! Then the files are staged and the caller continues the rebase through git's
//! own `commit`, so the hall's hooks run, with the [`TRAILER`] on the commit.
//! Any failed check is a [`Attempt::Failed`] and the caller aborts the rebase.

use std::collections::BTreeMap;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::Ctx;
use crate::action::agent::notify;
use crate::action::agent::run::{HostDeps, run_spec};
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};
use crate::domain::agent::spec::{AgentFrontmatter, AgentSpec, NotifyTarget, Runtime, WritePath};
use crate::domain::agent::tool::Tool;
use crate::domain::memory_page::PageIssueCode;
use crate::domain::provider::Provider;
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::git::{CommitInfo, Conflict, Git};
use crate::infra::{fs, hash};
use crate::store;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::page_source::parse_page;

use super::read_manifest;

/// The agent's name: its receipts are filed under it and the commit trailer
/// names it.
pub(crate) const AGENT: &str = "hall-sync-resolver";

/// Added to the message of every commit this agent resolved a conflict in.
pub(crate) const TRAILER: &str = "Resolved-by: ivar hall-sync-resolver";

/// How long one run may take.
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What of the two sides' text goes in the prompt, in all. A prompt is one
/// argument to the tool and the operating system caps those; a file that does
/// not fit is still there, with git's markers, for the agent to read.
const PROMPT_BUDGET: usize = 80_000;
const MIN_SIDE: usize = 2_000;

/// The machine the agent runs on, which tests replace.
pub(crate) struct Env<'a> {
    /// What the run needs from the host: the tool's login, the launcher that
    /// confines it.
    pub host: &'a dyn Fn() -> Result<HostDeps, Failure>,
    /// How long one run may take.
    pub timeout: Duration,
}

impl Env<'static> {
    /// This machine.
    pub(crate) fn system() -> Self {
        Self {
            host: &HostDeps::system,
            timeout: TIMEOUT,
        }
    }
}

/// How one attempt to settle conflicts with the agent ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// Every file is resolved, checked and staged; the rebase can continue.
    Resolved(Vec<Utf8PathBuf>),
    /// The agent was not asked: the hall switched it off.
    Declined,
    /// The agent could not settle them, or what it wrote did not pass. The
    /// string says why. The files are in an unknown state: the caller aborts.
    Failed(String),
}

/// Ask the agent to settle `conflicts`, check the result and stage it.
///
/// `conflicts` are the ones the fixed rules left, in a hall whose rebase is
/// stopped on them.
pub(crate) fn resolve(
    layout: &Layout,
    git: &impl Git,
    env: &Env<'_>,
    conflicts: &[Conflict],
) -> Attempt {
    let manifest = match read_manifest(layout) {
        Ok(manifest) => manifest,
        Err(failure) => {
            return Attempt::Failed(format!("could not read ivar.json: {}", failure.what));
        }
    };
    if !manifest.auto_resolve() {
        return Attempt::Declined;
    }
    match attempt(layout, git, env, &manifest, conflicts) {
        Ok(files) => Attempt::Resolved(files),
        Err(why) => Attempt::Failed(why),
    }
}

fn attempt(
    layout: &Layout,
    git: &impl Git,
    env: &Env<'_>,
    manifest: &Manifest,
    conflicts: &[Conflict],
) -> Result<Vec<Utf8PathBuf>, String> {
    let root = layout.root();
    let tool = tool_for(manifest).ok_or_else(|| {
        "none of the hall's providers has an agent tool to run it with".to_owned()
    })?;
    let paths: Vec<Utf8PathBuf> = conflicts.iter().map(|c| c.path.clone()).collect();
    let prompt = prompt(root, git, conflicts)
        .map_err(|why| format!("could not read the conflict: {why}"))?;
    let spec = spec(tool, env.timeout, &paths, &prompt)?;

    let before = snapshot(git, root, &paths)?;
    let report = run_spec(
        &Ctx::new(root),
        layout,
        &spec,
        Trigger::Manual,
        env.host,
        &|_| true,
    )
    .map_err(|failure| format!("{AGENT} could not run: {}", failure.what))?;
    let receipt = report.value;
    match receipt.outcome {
        AgentRunOutcome::Succeeded => {}
        AgentRunOutcome::TimedOut => {
            return Err(format!(
                "{AGENT} did not finish within {}s",
                env.timeout.as_secs()
            ));
        }
        other => {
            return Err(format!(
                "{AGENT} {}: {}",
                other.as_str(),
                last_line(&receipt.output)
            ));
        }
    }

    validate(git, root, &before, &paths)?;
    git.stage_paths(root, &paths)
        .map_err(|error| format!("could not stage the resolved files: {error}"))?;
    Ok(paths)
}

/// The tool the hall's default provider runs agents with, or the first
/// available provider that has one.
fn tool_for(manifest: &Manifest) -> Option<Tool> {
    let providers = manifest.providers();
    std::iter::once(providers.default_provider())
        .chain(providers.available().iter().copied())
        .find_map(tool_of)
}

const fn tool_of(provider: Provider) -> Option<Tool> {
    match provider {
        Provider::ClaudeCode | Provider::T3 => Some(Tool::Claude),
        Provider::OpenCode => Some(Tool::Opencode),
        Provider::Omp => None,
    }
}

/// The embedded spec: a host run of `tool`, with no files to write yet.
fn base_spec(tool: Tool, prompt: &str) -> Result<AgentSpec, String> {
    let frontmatter = AgentFrontmatter {
        // A spec needs a timing; this one is never scheduled.
        schedule: Some("@daily".to_owned()),
        runtime: Runtime::Host,
        tool: Some(tool.as_str().to_owned()),
        ..AgentFrontmatter::default()
    };
    AgentSpec::from_parts(AGENT, frontmatter, prompt).map_err(|error| {
        format!(
            "the {AGENT} spec is invalid: `{}`: {}",
            error.field, error.reason
        )
    })
}

/// The embedded spec, allowed to write `files` and nothing else.
fn spec(
    tool: Tool,
    timeout: Duration,
    files: &[Utf8PathBuf],
    prompt: &str,
) -> Result<AgentSpec, String> {
    let mut spec = base_spec(tool, prompt)?;
    spec.timeout = timeout;
    // `Hall` directly, not parsed from text: a file called `~/x` is a path in
    // the hall, and parsing would read it as one under the home directory.
    spec.write = files.iter().cloned().map(WritePath::Hall).collect();
    Ok(spec)
}

// -- the prompt -------------------------------------------------------------

fn prompt(root: &Utf8Path, git: &impl Git, conflicts: &[Conflict]) -> Result<String, String> {
    let remote = git.commit_info(root, "HEAD").map_err(|e| e.to_string())?;
    let local = git
        .commit_info(root, "REBASE_HEAD")
        .map_err(|e| e.to_string())?;
    let side = (PROMPT_BUDGET / (2 * conflicts.len().max(1))).max(MIN_SIDE);

    let mut text = format!(
        "You settle one stopped `git rebase` in a hall: a git repository of Markdown pages, memory \
         notes and docs that more than one machine writes.\n\n\
         The rebase is replaying a LOCAL commit on top of the REMOTE branch, and the files below \
         conflict.\n\
         - REMOTE: {}\n\
         - LOCAL: {}\n\n\
         Rules:\n\
         - Edit each file listed below in place, with its absolute path, and no other file. You \
         cannot write anything else.\n\
         - Keep the content of both sides. Never discard one side of a file of prose, and never \
         shorten a page. When both sides changed the same passage, keep what each of them \
         said.\n\
         - The file now holds git's conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`). Replace \
         the whole file with the merged result: no marker may remain.\n\
         - A page with YAML frontmatter keeps exactly one `---` block at the top, and it stays \
         valid. Merge list keys such as `tags` and `relations` as a union; keep one value for \
         a key with a single value.\n\
         - Do not run git, and do not stage or commit anything: ivar does that after it checks \
         your work.\n\
         When you are done, print one line saying what you did.\n",
        describe(&remote),
        describe(&local),
    );
    for conflict in conflicts {
        text.push_str("\n## ");
        text.push_str(conflict.path.as_str());
        text.push_str("\nPath: ");
        text.push_str(root.join(&conflict.path).as_str());
        text.push('\n');
        for (label, blob) in [("REMOTE", &conflict.ours), ("LOCAL", &conflict.theirs)] {
            text.push_str("\n### ");
            text.push_str(label);
            text.push_str(" version\n");
            match blob {
                None => text.push_str("(this side deleted the file)\n"),
                Some(blob) => {
                    let content = git.blob_text(root, blob).map_err(|e| e.to_string())?;
                    let (content, cut) = clip(&content, side);
                    text.push_str(&fenced(content));
                    if cut {
                        text.push_str(
                            "(cut here: read the file itself for the rest of both sides)\n",
                        );
                    }
                }
            }
        }
    }
    Ok(text)
}

fn describe(commit: &CommitInfo) -> String {
    let short: String = commit.sha.chars().take(10).collect();
    format!("commit {short} \"{}\"", commit.subject)
}

/// `text` as a Markdown code block whose fence is longer than any run of
/// backticks inside it.
fn fenced(text: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{text}\n{fence}\n")
}

/// At most `max` bytes of `text`, cut at a character boundary.
fn clip(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text.get(..end).unwrap_or_default(), true)
}

// -- the checks -------------------------------------------------------------

/// What the hall looks like outside the files the agent may write: every path
/// git reports as changed, with a hash of what is on disk (`None`: gone).
type Snapshot = BTreeMap<Utf8PathBuf, Option<String>>;

fn snapshot(git: &impl Git, root: &Utf8Path, allowed: &[Utf8PathBuf]) -> Result<Snapshot, String> {
    let changed = git
        .changed_paths(root)
        .map_err(|error| format!("could not read the hall's changes: {error}"))?;
    Ok(changed
        .into_iter()
        .filter(|path| !allowed.contains(path))
        .map(|path| {
            let digest = fs::read_bytes(&root.join(&path))
                .ok()
                .flatten()
                .map(|bytes| hash::bytes(&bytes));
            (path, digest)
        })
        .collect())
}

/// Everything that must hold before the resolved files are staged.
fn validate(
    git: &impl Git,
    root: &Utf8Path,
    before: &Snapshot,
    files: &[Utf8PathBuf],
) -> Result<(), String> {
    let after = snapshot(git, root, files)?;
    let mut touched: Vec<&str> = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .map(|path| path.as_str())
        .collect();
    touched.sort_unstable();
    touched.dedup();
    if !touched.is_empty() {
        return Err(format!(
            "{AGENT} changed files that were not in conflict: {}",
            touched.join(", ")
        ));
    }

    for file in files {
        let Some(bytes) = fs::read_bytes(&root.join(file))
            .map_err(|error| format!("could not read {file} back: {error}"))?
        else {
            // A file the agent removed has nothing left to check.
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if let Some(line) = marker_line(&text) {
            return Err(format!(
                "{AGENT} left a conflict marker in {file}, at line {line}"
            ));
        }
        if is_memory_page(file) {
            let broken = broken_page(file, &text);
            if !broken.is_empty() {
                return Err(format!(
                    "{AGENT} left {file} as an invalid memory page: {}",
                    broken.join("; ")
                ));
            }
        }
    }
    Ok(())
}

/// What is wrong with the memory page at `path`, as the memory lint reads it.
///
/// Only what the file's content decides: its frontmatter, the kind it claims
/// and its `expires_at`. Where the page lives and which keys it uses are not
/// the agent's doing, and a page that was already named oddly would otherwise
/// block every resolution.
fn broken_page(path: &Utf8Path, text: &str) -> Vec<String> {
    parse_page(path.as_str(), text)
        .err()
        .into_iter()
        .flatten()
        .filter(|issue| {
            matches!(
                issue.code,
                PageIssueCode::FrontmatterInvalid
                    | PageIssueCode::KindMismatch
                    | PageIssueCode::ExpiresAtInvalid
            )
        })
        .map(|issue| issue.message)
        .collect()
}

/// The 1-based line of the first conflict marker in `text`, if any: a line
/// that starts with seven `<` or seven `>` followed by a space or nothing, or
/// that is seven `=` and nothing else.
fn marker_line(text: &str) -> Option<usize> {
    text.lines().position(is_marker).map(|index| index + 1)
}

fn is_marker(line: &str) -> bool {
    let line = line.trim_end_matches('\r');
    let opens = |mark: char| {
        line.strip_prefix(&mark.to_string().repeat(7))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    };
    opens('<') || opens('>') || line == "======="
}

/// Whether `path` is a page the memory lint reads.
fn is_memory_page(path: &Utf8Path) -> bool {
    path.starts_with("docs/memory")
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// The last non-empty line of `output`, which is where a tool says how it
/// ended.
fn last_line(output: &str) -> String {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .chars()
        .take(300)
        .collect()
}

// -- telling someone ----------------------------------------------------------

/// The `notify` target the hall's agents share, which is the hall's channel:
/// there is no hall-wide setting, so the first agent in name order that has
/// one lends it. `None` when no agent does.
fn hall_notify(layout: &Layout) -> Option<NotifyTarget> {
    store::agent::list_files(layout)
        .ok()?
        .iter()
        .find_map(|file| store::agent::parse(file).ok()?.notify)
}

/// Tell the hall's `notify` channel, when it has one, which files the agent
/// resolved, for a person to review. `Err` when the delivery failed.
pub(crate) fn notify_resolved(layout: &Layout, files: &[Utf8PathBuf]) -> Result<(), String> {
    let Some(target) = hall_notify(layout) else {
        return Ok(());
    };
    let mut spec = base_spec(Tool::Claude, "")?;
    spec.notify = Some(target);

    let now = rfc3339_now();
    let mut receipt = AgentRunReceipt::skipped(
        crate::domain::agent::receipt::run_id(&now, "sync"),
        AGENT.to_owned(),
        Trigger::Manual,
        now.clone(),
    );
    receipt.outcome = AgentRunOutcome::Succeeded;
    receipt.output = format!(
        "ivar settled hall rebase conflicts with {AGENT}. Review what it merged:\n{}",
        files
            .iter()
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    notify::deliver(layout, &spec, &mut receipt);
    receipt.notify_error.map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/hall/resolver.rs"]
mod tests;
