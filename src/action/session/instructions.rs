//! Repository instructions delivered with the tool calls that touch a linked repo.
//!
//! Pure over the filesystem: paths are joined lexically and never
//! canonicalised, so the view dir's repo symlink decides which branch's file
//! is read.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "unused outside tests until the guard calls it")
)]

use std::collections::BTreeMap;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::infra::{fs, hash};
use crate::providers::search::command_segments;

/// Most paths one glob word may expand to.
pub(crate) const GLOB_CAP: usize = 50;

/// Paths a tool call names: `file_path`/`path`/`filePath`, the `workdir`/`cwd`
/// field, and the words of `command`. The command base (the `workdir`/`cwd`
/// field, else `cwd`) always counts and comes last.
pub(crate) fn touched_paths(tool_input: &serde_json::Value, cwd: &Utf8Path) -> Vec<Utf8PathBuf> {
    let field = |key: &str| {
        tool_input
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let mut out: Vec<Utf8PathBuf> = ["file_path", "path", "filePath"]
        .into_iter()
        .filter_map(field)
        .map(|raw| lexical(cwd, raw))
        .collect();
    let base = ["workdir", "cwd"]
        .into_iter()
        .find_map(field)
        .map_or_else(|| cwd.to_owned(), |workdir| lexical(cwd, workdir));
    if let Some(command) = field("command") {
        shell_paths(command, &base, &mut out);
    }
    out.push(base);
    let mut unique = Vec::with_capacity(out.len());
    for path in out {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

/// `raw` joined onto `base` (an absolute `raw` replaces it), with `.` and `..`
/// resolved by name. Never touches the filesystem.
fn lexical(base: &Utf8Path, raw: &str) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in base.join(raw).components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Paths named by the words of a shell command. `cd` moves the base for the
/// words after it; `--opt=value` counts its value; flags are skipped, so the
/// directory after `-C` counts as an ordinary word.
fn shell_paths(command: &str, base: &Utf8Path, out: &mut Vec<Utf8PathBuf>) {
    let mut cwd = base.to_owned();
    for segment in command_segments(command) {
        let words = words(segment);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        if program == "cd" {
            if let Some(target) = args.first().filter(|word| literal(word)) {
                cwd = lexical(&cwd, target);
                out.push(cwd.clone());
            }
            continue;
        }
        for arg in args {
            let word = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with('-') => value,
                _ => arg.as_str(),
            };
            if word.is_empty() || word.starts_with('-') || !literal(word) {
                continue;
            }
            if word.contains(['*', '?', '[']) {
                out.extend(expand_glob(&cwd, word));
            } else {
                out.push(lexical(&cwd, word));
            }
        }
    }
}

/// A word whose value the shell would not compute: no `$`, no backtick, no `~`.
fn literal(word: &str) -> bool {
    !word.contains(['$', '`']) && !word.starts_with('~')
}

/// Split one command segment into words: whitespace, `(`, `)`, `<` and `>`
/// separate outside quotes; quotes are removed; `\` escapes outside single quotes.
fn words(segment: &str) -> Vec<String> {
    let (mut out, mut word) = (Vec::new(), String::new());
    let (mut quote, mut escaped) = (None::<char>, false);
    for c in segment.chars() {
        if escaped {
            word.push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some('"') | None, '\\') => escaped = true,
            (Some(_), _) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, _) if c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>') => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            (None, _) => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Expand a glob word the way the shell would, through the filesystem, at most
/// `GLOB_CAP` paths. A leading `.` must be matched explicitly. No match yields
/// the directory before the first wildcard component.
fn expand_glob(cwd: &Utf8Path, word: &str) -> Vec<Utf8PathBuf> {
    let (mut hits, mut prefix, mut globbed) = (vec![Utf8PathBuf::new()], Utf8PathBuf::new(), false);
    for component in lexical(cwd, word).components() {
        let part = component.as_str();
        if !part.contains(['*', '?', '[']) {
            for hit in &mut hits {
                hit.push(part);
            }
            if !globbed {
                prefix.push(part);
            }
            continue;
        }
        globbed = true;
        let pattern: Vec<char> = part.chars().collect();
        hits = hits
            .iter()
            .flat_map(|dir| fs::read_dir(dir).unwrap_or_default())
            .filter(|entry| {
                entry.file_name().is_some_and(|name| {
                    (part.starts_with('.') || !name.starts_with('.'))
                        && wildcard(&pattern, &name.chars().collect::<Vec<_>>())
                })
            })
            .take(GLOB_CAP)
            .collect();
    }
    if hits.is_empty() { vec![prefix] } else { hits }
}

/// Shell wildcard match of one path component: `*`, `?`, `[...]` (with `!`/`^`
/// negation and `a-z` ranges). An unclosed `[` matches itself.
fn wildcard(pattern: &[char], name: &[char]) -> bool {
    match (pattern.split_first(), name.split_first()) {
        (None, None) => true,
        (Some(('*', rest)), _) => {
            wildcard(rest, name)
                || name
                    .split_first()
                    .is_some_and(|(_, tail)| wildcard(pattern, tail))
        }
        (Some(('?', rest)), Some((_, tail))) => wildcard(rest, tail),
        (Some(('[', rest)), Some((&c, tail))) => match class(rest, c) {
            Some((hit, after)) => hit && wildcard(after, tail),
            None => c == '[' && wildcard(rest, tail),
        },
        (Some((p, rest)), Some((c, tail))) => p == c && wildcard(rest, tail),
        _ => false,
    }
}

/// The `[...]` class whose body starts at `pattern` (just after `[`): whether
/// `c` is in it, and the pattern after the closing `]`. `None` when unclosed.
fn class(pattern: &[char], c: char) -> Option<(bool, &[char])> {
    let (negated, body) = match pattern.split_first() {
        Some(('!' | '^', rest)) => (true, rest),
        _ => (false, pattern),
    };
    let close = body.iter().skip(1).position(|&x| x == ']')? + 1;
    let (mut items, after) = body.split_at(close);
    let mut hit = false;
    while let Some((&lo, rest)) = items.split_first() {
        if let ['-', hi, tail @ ..] = rest {
            hit |= (lo..=*hi).contains(&c);
            items = tail;
        } else {
            hit |= lo == c;
            items = rest;
        }
    }
    Some((hit != negated, after.get(1..)?))
}

/// Instruction files from the repo root down to `path`'s deepest existing
/// directory, root first, one per directory (provider-native name, else the
/// other). Empty when `path` is not under `<view_dir>/<repo>/` for a
/// directory `<repo>` present in the view whose name does not start with `.`.
/// Lexical: never canonicalises, so the view symlink decides the branch.
pub(crate) fn instruction_chain(
    view_dir: &Utf8Path,
    path: &Utf8Path,
    provider: Provider,
) -> Vec<Utf8PathBuf> {
    let Ok(relative) = path.strip_prefix(view_dir) else {
        return Vec::new();
    };
    let Some(Utf8Component::Normal(name)) = relative.components().next() else {
        return Vec::new();
    };
    let repo = view_dir.join(name);
    if name.starts_with('.') || !fs::is_dir(&repo).unwrap_or(false) {
        return Vec::new();
    }
    let mut dir = path.to_owned();
    while dir != repo && !fs::is_dir(&dir).unwrap_or(false) {
        if !dir.pop() {
            return Vec::new();
        }
    }
    let names = [provider.instruction_file(), fallback_file(provider)];
    let mut chain = Vec::new();
    loop {
        if let Some(file) = names
            .iter()
            .map(|name| dir.join(name))
            .find(|file| fs::is_file(file).unwrap_or(false))
        {
            chain.push(file);
        }
        if dir == repo || !dir.pop() {
            break;
        }
    }
    chain.reverse();
    chain
}

/// The instruction file read when a directory has no provider-native one.
const fn fallback_file(provider: Provider) -> &'static str {
    match provider {
        Provider::ClaudeCode => "AGENTS.md",
        Provider::OpenCode | Provider::Omp => "CLAUDE.md",
    }
}

/// Where delivery state lives: inside the view (the omp sandbox only allows
/// writes there) but under the ivar-owned config dir, never the view root.
pub(crate) fn state_dir(view_dir: &Utf8Path, provider: Provider) -> Utf8PathBuf {
    view_dir
        .join(provider.config_dir())
        .join("ivar")
        .join("instructions")
}

/// Unseen-or-changed instruction files for `agent`, whole, joined with a blank
/// line; records them in `<state_dir>/<agent>.json` (path -> sha256 hex prefix
/// 16) under `<agent>.lock`.
///
/// # Errors
///
/// A [`Failure`] when the state dir, lock or state file cannot be created,
/// read or written. An unreadable instruction file is skipped, not an error.
pub(crate) fn deliver(
    view_dir: &Utf8Path,
    provider: Provider,
    agent: &str,
    paths: &[Utf8PathBuf],
) -> Result<Option<String>, Failure> {
    let files = instruction_files(view_dir, provider, paths);
    if files.is_empty() {
        return Ok(None);
    }
    record(view_dir, provider, agent, &files)
}

/// The chains of every path, merged in order without repeats.
fn instruction_files(
    view_dir: &Utf8Path,
    provider: Provider,
    paths: &[Utf8PathBuf],
) -> Vec<Utf8PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        for file in instruction_chain(view_dir, path, provider) {
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
}

/// Render the files `agent` has not seen in their current content and record
/// them as seen. `Ok(None)` when there is nothing new.
fn record(
    view_dir: &Utf8Path,
    provider: Provider,
    agent: &str,
    files: &[Utf8PathBuf],
) -> Result<Option<String>, Failure> {
    let dir = state_dir(view_dir, provider);
    fs::ensure_dir(&dir)?;
    let key = file_key(agent);
    let _lock = fs::lock_exclusive(&dir.join(format!("{key}.lock")))?;
    let state_file = dir.join(format!("{key}.json"));
    // A corrupt state file re-delivers everything rather than failing.
    let mut seen: BTreeMap<String, String> = fs::read_text(&state_file)?
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut sections = Vec::new();
    for file in files {
        let Ok(Some(body)) = fs::read_text(file) else {
            continue;
        };
        let digest = hash::text(&body);
        let digest = digest.get(..16).unwrap_or(&digest).to_owned();
        let previous = seen.insert(file.to_string(), digest.clone());
        if previous.as_ref() == Some(&digest) {
            continue;
        }
        let updated = if previous.is_some() { "UPDATED " } else { "" };
        let scope = file.parent().unwrap_or(view_dir);
        sections.push(format!(
            "{updated}Repository instructions from {file} (they apply to work under {scope}):\n{body}"
        ));
    }
    if sections.is_empty() {
        return Ok(None);
    }
    let state = serde_json::to_string(&seen).map_err(|error| {
        Failure::failed(
            "session.instructions_state",
            format!("could not encode the instruction delivery state: {error}"),
        )
    })?;
    fs::write_atomic(&state_file, state.as_bytes())?;
    Ok(Some(sections.join("\n\n")))
}

/// `raw` safe as a file name: every char outside `[A-Za-z0-9_.-]` becomes `_`.
fn file_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/instructions.rs"]
mod tests;
