//! Shell commands in a plan's fenced blocks, with the directory each runs in.
//!
//! Deliberately conservative: a false positive blocks plan approval, so
//! anything that is not a literal path or script name stops the scan of its
//! block instead of being reported.

use camino::Utf8PathBuf;

const SHELL_FENCES: &[&str] = &["sh", "bash", "shell", "console"];
/// `yarn run` also runs package binaries, so a missing script proves nothing.
const RUNNERS: &[&str] = &["npm", "pnpm"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    pub line: usize,
    pub command: String,
    pub kind: CommandKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    ChangeDir { dir: Utf8PathBuf },
    RunScript { dir: Utf8PathBuf, script: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFinding {
    pub file: Utf8PathBuf,
    pub line: usize,
    pub command: String,
    pub reason: String,
}

impl CommandFinding {
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{}:{}: `{}` — {}",
            self.file, self.line, self.command, self.reason
        )
    }
}

enum Fence {
    Outside,
    Other(Marker),
    /// `None` once a command made the directory in effect unknowable.
    Shell(Marker, Option<Utf8PathBuf>),
}

#[derive(Clone, Copy)]
struct Marker {
    char: char,
    len: usize,
}

impl Marker {
    fn parse(line: &str) -> Option<(Self, &str)> {
        let char = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let len = line.chars().take_while(|&c| c == char).count();
        (len >= 3).then(|| (Self { char, len }, &line[len..]))
    }

    fn closes(self, opener: Self) -> bool {
        self.char == opener.char && self.len >= opener.len
    }
}

/// `repos` are the declared repo names: a `cd` into one of them from the hall
/// root cannot be resolved against a single repo's worktree.
#[must_use]
pub fn scan_shell_commands(source: &str, repos: &[String]) -> Vec<ShellCommand> {
    let mut commands = Vec::new();
    let mut fence = Fence::Outside;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if let Some((marker, info)) = Marker::parse(line) {
            let next = match &fence {
                Fence::Outside if SHELL_FENCES.contains(&info.trim()) => {
                    Some(Fence::Shell(marker, Some(Utf8PathBuf::new())))
                }
                Fence::Outside => Some(Fence::Other(marker)),
                Fence::Other(opener) | Fence::Shell(opener, _) => {
                    marker.closes(*opener).then_some(Fence::Outside)
                }
            };
            if let Some(next) = next {
                fence = next;
                continue;
            }
        }
        let Fence::Shell(_, cwd) = &mut fence else {
            continue;
        };
        let text = line.strip_prefix("$ ").unwrap_or(line);
        for segment in text.split("&&").flat_map(|part| part.split(';')) {
            let Some(dir) = cwd.as_mut() else {
                break;
            };
            let words: Vec<&str> = segment.split_whitespace().collect();
            let command = segment.trim().to_owned();
            match words.as_slice() {
                ["cd", target] if is_literal_repo_path(target, repos) => {
                    dir.push(target.trim_start_matches("./"));
                    commands.push(ShellCommand {
                        line: index + 1,
                        command,
                        kind: CommandKind::ChangeDir { dir: dir.clone() },
                    });
                }
                ["cd", ..] => *cwd = None,
                [runner, "run", script, ..]
                    if RUNNERS.contains(runner)
                        && is_literal(script)
                        && !script.starts_with('-')
                        && !words.iter().any(|word| is_scope_flag(word)) =>
                {
                    commands.push(ShellCommand {
                        line: index + 1,
                        command,
                        kind: CommandKind::RunScript {
                            dir: dir.clone(),
                            script: (*script).to_owned(),
                        },
                    });
                }
                _ => {}
            }
        }
    }
    commands
}

/// Flags that make the runner resolve the script somewhere other than the
/// current directory.
fn is_scope_flag(word: &str) -> bool {
    const SCOPE_FLAGS: &[&str] = &[
        "-w",
        "--workspace",
        "--filter",
        "-C",
        "--prefix",
        "-r",
        "--recursive",
    ];
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    SCOPE_FLAGS.contains(&name)
}

fn is_literal(word: &str) -> bool {
    !word.is_empty() && !word.contains(['$', '*', '?', '~', '<', '>', '`', '\'', '"', '\\'])
}

fn is_literal_repo_path(path: &str, repos: &[String]) -> bool {
    let path = path.trim_start_matches("./");
    let first = path.split('/').next().unwrap_or_default();
    is_literal(path)
        && !path.starts_with(['/', '-'])
        && !path.starts_with(".ivar")
        && !repos.iter().any(|repo| repo == first)
        && !path.split('/').any(|component| component == "..")
}

#[cfg(test)]
#[path = "../../tests/unit/domain/plan_commands.rs"]
mod tests;
