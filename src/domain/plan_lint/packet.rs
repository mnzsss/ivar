//! The per-packet rules: what one packet's text and file list must satisfy.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::Failure;

use super::{
    CREATE_EXISTS, Finding, HOOK_LIMIT_BYTES, MODIFY_MISSING, NUMBER_PLACEHOLDER, PLACEHOLDER,
    PacketSource, READERS_MISSING, READERS_WITHOUT_OUTPUT, RepoFiles, SKETCH_WITHOUT_REASON,
    TEST_IN_OVERSIZED_FILE,
};

/// Paths other packets create, by path, so a `Modify:` of a file an earlier
/// packet introduces is not mistaken for a file that does not exist.
pub(super) type Created = BTreeMap<String, BTreeSet<String>>;

/// Fewer words than this cannot state why literal code is inappropriate.
const MIN_SKETCH_REASON_WORDS: usize = 3;

/// One line of a packet, with whether a fenced block covers it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Line<'a> {
    pub no: usize,
    pub text: &'a str,
    /// Inside a fenced block, or the fence line itself.
    pub fenced: bool,
    /// The opening or closing fence line.
    pub delimiter: bool,
}

fn fence_marker(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|c| *c == first).count();
    (len >= 3).then_some((first, len))
}

/// Split `source` into lines, tracking fenced blocks.
pub(super) fn lines(source: &str) -> Vec<Line<'_>> {
    let mut open: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (index, text) in source.lines().enumerate() {
        let trimmed = text.trim_start();
        let marker = fence_marker(trimmed);
        let (fenced, delimiter) = match (open, marker) {
            (None, Some(found)) => {
                open = Some(found);
                (true, true)
            }
            (Some((char, len)), Some((c, n)))
                if c == char && n >= len && trimmed.trim_start_matches(c).trim().is_empty() =>
            {
                open = None;
                (true, true)
            }
            (Some(_), _) => (true, false),
            (None, None) => (false, false),
        };
        out.push(Line {
            no: index + 1,
            text,
            fenced,
            delimiter,
        });
    }
    out
}

fn is_label_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("**") && trimmed.contains(":**")
}

fn is_step_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("- [") && trimmed.contains("**Step")
}

/// The lines under `**<label>:**`, from the label line to the next label or
/// step line outside a fenced block. Empty when the packet has no such label.
pub(super) fn section<'a, 'b>(lines: &'b [Line<'a>], label: &str) -> &'b [Line<'a>] {
    let marker = format!("**{label}:**");
    let Some(start) = lines
        .iter()
        .position(|line| !line.fenced && line.text.trim_start().starts_with(&marker))
    else {
        return &[];
    };
    let tail = lines.get(start..).unwrap_or_default();
    let end = tail
        .iter()
        .skip(1)
        .position(|line| !line.fenced && (is_label_line(line.text) || is_step_line(line.text)))
        .map_or(tail.len(), |offset| offset + 1);
    tail.get(..end).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Create,
    Modify,
    Test,
}

struct Entry {
    kind: Kind,
    path: String,
    line: usize,
}

/// A `- Create: \`path\`` bullet. A trailing `:12-40` range is not part of
/// the path.
fn parse_entry(line: &Line<'_>) -> Option<Entry> {
    let bullet = line
        .text
        .trim_start()
        .strip_prefix("- ")
        .or_else(|| line.text.trim_start().strip_prefix("* "))?;
    let (word, rest) = bullet.split_once(':')?;
    let kind = match word.trim().to_ascii_lowercase().as_str() {
        "create" => Kind::Create,
        "modify" => Kind::Modify,
        "test" => Kind::Test,
        _ => return None,
    };
    let rest = rest.trim();
    let raw = match rest.strip_prefix('`') {
        Some(inner) => inner.split('`').next()?,
        None => rest.split_whitespace().next()?,
    };
    let path = match raw.rsplit_once(':') {
        Some((head, tail))
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            head
        }
        _ => raw,
    };
    (!path.is_empty() && !path.ends_with('/')).then(|| Entry {
        kind,
        path: path.to_owned(),
        line: line.no,
    })
}

fn entries(lines: &[Line<'_>]) -> Vec<Entry> {
    section(lines, "Files")
        .iter()
        .filter(|line| !line.fenced)
        .filter_map(parse_entry)
        .collect()
}

pub(super) fn created_paths_of(packets: &[PacketSource]) -> Created {
    let mut created = Created::new();
    for source in packets {
        for entry in entries(&lines(&source.text)) {
            if entry.kind == Kind::Create {
                created
                    .entry(entry.path)
                    .or_default()
                    .insert(source.file.clone());
            }
        }
    }
    created
}

/// The text of the `Consumes:` bullet in the packet's Interfaces section,
/// through to its `Produces:` bullet.
pub(super) fn consumes_text(source: &str) -> String {
    let all = lines(source);
    let mut taking = false;
    let mut out = String::new();
    for line in section(&all, "Interfaces") {
        let trimmed = line.text.trim_start().trim_start_matches(['-', '*', ' ']);
        if let Some(rest) = trimmed.strip_prefix("Consumes:") {
            taking = true;
            out.push_str(rest);
        } else if trimmed.starts_with("Produces:") {
            taking = false;
        } else if taking {
            out.push(' ');
            out.push_str(line.text);
        }
    }
    out
}

pub(super) fn check(
    source: &PacketSource,
    files: Option<&dyn RepoFiles>,
    created: &Created,
) -> Result<Vec<Finding>, Failure> {
    let all = lines(&source.text);
    let mut findings = Vec::new();
    let mut found = |line: usize, code: &'static str, what: String| {
        findings.push(Finding {
            file: source.file.clone(),
            line,
            code,
            what,
        });
    };

    for line in &all {
        if let Some((code, what)) = placeholder(line.text) {
            found(line.no, code, what);
        }
        if !line.fenced
            && let Some(what) = weak_sketch(line.text)
        {
            found(line.no, SKETCH_WITHOUT_REASON, what);
        }
    }
    readers(&all, &mut found);
    if let Some(files) = files {
        file_rules(&all, files, created, &source.file, &mut found)?;
    }
    findings.sort_by_key(|finding| finding.line);
    Ok(findings)
}

fn readers(all: &[Line<'_>], found: &mut impl FnMut(usize, &'static str, String)) {
    let body = section(all, "Readers");
    let Some(head) = body.first() else {
        found(
            0,
            READERS_MISSING,
            "the packet has no `**Readers:**` section".to_owned(),
        );
        return;
    };
    let has_output = body
        .iter()
        .any(|line| line.fenced && !line.delimiter && !line.text.trim().is_empty());
    if !has_output {
        found(
            head.no,
            READERS_WITHOUT_OUTPUT,
            "Readers holds no fenced block: paste the real `git grep -n` output, or the command that found nothing"
                .to_owned(),
        );
    }
}

fn weak_sketch(text: &str) -> Option<String> {
    let (_, reason) = text.split_once("**Sketch:**")?;
    let reason = reason.trim_matches(|c: char| !c.is_alphanumeric());
    (reason.split_whitespace().count() < MIN_SKETCH_REASON_WORDS).then(|| {
        "`**Sketch:**` must state why literal code is inappropriate for that volume".to_owned()
    })
}

fn file_rules(
    all: &[Line<'_>],
    files: &dyn RepoFiles,
    created: &Created,
    packet_file: &str,
    found: &mut impl FnMut(usize, &'static str, String),
) -> Result<(), Failure> {
    let mut checked_for_tests = BTreeSet::new();
    for entry in entries(all) {
        let size = files.len(&entry.path)?;
        match (entry.kind, size) {
            (Kind::Create, Some(_)) => found(
                entry.line,
                CREATE_EXISTS,
                format!(
                    "`Create: {}` names a file that already exists: use `Modify:` with its size",
                    entry.path
                ),
            ),
            (Kind::Modify, None) => {
                let by_other = created
                    .get(&entry.path)
                    .is_some_and(|owners| owners.iter().any(|owner| owner != packet_file));
                if !by_other {
                    found(
                        entry.line,
                        MODIFY_MISSING,
                        format!(
                            "`Modify: {}` names a file that does not exist: use `Create:`",
                            entry.path
                        ),
                    );
                }
            }
            _ => {}
        }
        let takes_tests = entry.kind == Kind::Test
            || (entry.kind == Kind::Modify && looks_like_test(&entry.path));
        if let Some(bytes) = size
            && takes_tests
            && bytes >= HOOK_LIMIT_BYTES
            && checked_for_tests.insert(entry.path.clone())
        {
            found(
                entry.line,
                TEST_IN_OVERSIZED_FILE,
                format!(
                    "`{}` is {bytes} bytes, at or above the {HOOK_LIMIT_BYTES}-byte hook limit: put the new test in a new file",
                    entry.path
                ),
            );
        }
    }
    Ok(())
}

fn looks_like_test(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or_default();
    lower
        .split('/')
        .any(|part| matches!(part, "tests" | "test" | "__tests__" | "spec"))
        || name.contains("test")
        || name.contains("spec")
}

/// The first placeholder on `text`, as `(code, what)`.
fn placeholder(text: &str) -> Option<(&'static str, String)> {
    if let Some(marker) = number_marker(text) {
        return Some((
            NUMBER_PLACEHOLDER,
            format!("`{marker}` was never filled in: write the real number"),
        ));
    }
    let hit = ["TODO", "TBD"]
        .into_iter()
        .find(|word| contains_word(text, word))
        .or_else(|| {
            ["todo!(", "unimplemented!("]
                .into_iter()
                .find(|stub| text.contains(stub))
        })
        .or_else(|| phrase(text));
    hit.map(|hit| {
        (
            PLACEHOLDER,
            format!("placeholder `{hit}`: write out what it stands for"),
        )
    })
}

const PHRASES: [&str; 5] = [
    "as in interfaces",
    "(run at execution)",
    "implement later",
    "similar to task",
    "see interfaces above",
];

fn phrase(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if let Some(found) = PHRASES.into_iter().find(|phrase| lower.contains(phrase)) {
        return Some(found);
    }
    let after = lower.split_once("check")?.1;
    after
        .contains("during execution")
        .then_some("check … during execution")
}

fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let after = text.get(at + word.len()..).and_then(|t| t.chars().next());
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
        edge(before) && edge(after)
    })
}

/// `<NNN>`: an angle-bracketed run of two or more capital Ns.
fn number_marker(text: &str) -> Option<String> {
    text.match_indices('<').find_map(|(at, _)| {
        let tail = text.get(at + 1..)?;
        let run = tail.chars().take_while(|c| *c == 'N').count();
        (run >= 2 && tail.get(run..)?.starts_with('>')).then(|| format!("<{}>", "N".repeat(run)))
    })
}
