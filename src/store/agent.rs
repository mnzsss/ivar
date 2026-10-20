//! Agent definitions under `<hall>/agents/` and their run receipts.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::agent::receipt::{AgentRunReceipt, RECEIPT_VERSION};
use crate::domain::agent::spec::{AgentFrontmatter, AgentSpec};
use crate::error::Failure;
use crate::infra::{frontmatter, fs};
use crate::store::layout::Layout;
use crate::store::versioned::{Policy, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    pub name: String,
    pub path: Utf8PathBuf,
    pub source: String,
}

fn entries(dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Failure> {
    if !fs::is_dir(dir)? {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(dir)?;
    paths.sort();
    Ok(paths)
}

fn stems_with(dir: &Utf8Path, extension: &str) -> Result<Vec<(String, Utf8PathBuf)>, Failure> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|path| path.extension() == Some(extension))
        .filter_map(|path| Some((path.file_stem()?.to_owned(), path)))
        .collect())
}

/// Every `<hall>/agents/*.md`, sorted by name; empty when the directory is missing.
///
/// # Errors
///
/// Returns a [`Failure`] when the directory or a file cannot be read.
pub fn list_files(layout: &Layout) -> Result<Vec<AgentFile>, Failure> {
    stems_with(&layout.agents_dir(), "md")?
        .into_iter()
        .filter_map(|(name, path)| match fs::read_text(&path) {
            Ok(Some(source)) => Some(Ok(AgentFile { name, path, source })),
            Ok(None) => None,
            Err(e) => Some(Err(e.into())),
        })
        .collect()
}

/// # Errors
///
/// Returns `agent.invalid` naming the file and the failing field.
pub fn parse(file: &AgentFile) -> Result<AgentSpec, Failure> {
    let invalid = |field: &str, reason: &str| {
        Failure::failed(
            "agent.invalid",
            format!("{}: `{field}`: {reason}", file.path),
        )
    };
    let fm: AgentFrontmatter = frontmatter::parse_at(&file.path, &file.source).map_err(|f| {
        // serde_saphyr reports deny_unknown_fields as "unknown field `x`, expected one of …".
        let field = f.what.split('`').nth(1).unwrap_or("frontmatter");
        invalid(field, &f.what)
    })?;
    let body = frontmatter::split(&file.source)?.body;
    AgentSpec::from_parts(&file.name, fm, body).map_err(|e| {
        invalid(e.field, &e.reason)
            .expected(format!("a valid `{}`", e.field))
            .actual(e.reason)
    })
}

/// # Errors
///
/// Returns `agent.not_found` when `<hall>/agents/<name>.md` is absent, or
/// `agent.invalid` when it does not validate.
pub fn load(layout: &Layout, name: &str) -> Result<AgentSpec, Failure> {
    let path = layout.agent_file(name);
    let Some(source) = fs::read_text(&path)? else {
        return Err(Failure::failed(
            "agent.not_found",
            format!("no agent `{name}` at {path}"),
        ));
    };
    parse(&AgentFile {
        name: name.to_owned(),
        path,
        source,
    })
}

fn receipt_store(path: Utf8PathBuf) -> Store<AgentRunReceipt> {
    Store::new(path, Vec::new(), RECEIPT_VERSION, Policy::Local)
}

/// # Errors
///
/// Returns a [`Failure`] when the receipt cannot be written.
pub fn write_receipt(layout: &Layout, receipt: &AgentRunReceipt) -> Result<(), Failure> {
    fs::ensure_dir(&layout.agent_runs_dir(&receipt.agent))?;
    receipt_store(layout.agent_run_receipt(&receipt.agent, &receipt.run_id)).write(receipt)?;
    Ok(())
}

/// # Errors
///
/// Returns a [`Failure`] when the receipt exists but cannot be read.
pub fn read_receipt(
    layout: &Layout,
    agent: &str,
    run_id: &str,
) -> Result<Option<AgentRunReceipt>, Failure> {
    Ok(receipt_store(layout.agent_run_receipt(agent, run_id)).read()?)
}

/// The receipt with the greatest run id: run ids start with a UTC timestamp.
///
/// # Errors
///
/// Returns a [`Failure`] when the runs directory or the receipt cannot be read.
pub fn latest_receipt(layout: &Layout, agent: &str) -> Result<Option<AgentRunReceipt>, Failure> {
    match stems_with(&layout.agent_runs_dir(agent), "json")?.pop() {
        Some((run_id, _)) => read_receipt(layout, agent, &run_id),
        None => Ok(None),
    }
}

/// # Errors
///
/// Returns a [`Failure`] when a runs directory or the receipt cannot be read.
pub fn find_receipt(layout: &Layout, run_id: &str) -> Result<Option<AgentRunReceipt>, Failure> {
    for agent_dir in entries(&layout.agent_state_dir().join("runs"))? {
        let Some(agent) = agent_dir.file_name() else {
            continue;
        };
        if let Some(receipt) = read_receipt(layout, agent, run_id)? {
            return Ok(Some(receipt));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "../../tests/unit/store/agent.rs"]
mod tests;
