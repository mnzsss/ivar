//! Repository instructions delivered with the tool calls that touch a linked repo.
//!
//! Pure over the filesystem: paths are joined lexically and never
//! canonicalised, so the view dir's repo symlink decides which branch's file
//! is read.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "unused outside tests until the guard calls it")
)]

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use crate::infra::fs;
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

#[cfg(test)]
#[path = "../../../tests/unit/action/session/instructions.rs"]
mod tests;
