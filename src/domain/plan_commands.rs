//! Shell commands in a plan's fenced blocks, with the directory each runs in.
//!
//! Deliberately conservative: a false positive blocks plan approval, so
//! anything that is not a literal path or script name stops the scan of its
//! block instead of being reported.

use camino::Utf8PathBuf;

const SHELL_FENCES: &[&str] = &["sh", "bash", "shell", "console"];
const RUNNERS: &[&str] = &["npm", "pnpm", "yarn"];

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
    Other,
    /// `None` once a command made the directory in effect unknowable.
    Shell(Option<Utf8PathBuf>),
}

#[must_use]
pub fn scan_shell_commands(source: &str) -> Vec<ShellCommand> {
    let mut commands = Vec::new();
    let mut fence = Fence::Outside;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if let Some(info) = line.strip_prefix("```") {
            fence = match fence {
                Fence::Outside if SHELL_FENCES.contains(&info.trim()) => {
                    Fence::Shell(Some(Utf8PathBuf::new()))
                }
                Fence::Outside => Fence::Other,
                Fence::Other | Fence::Shell(_) => Fence::Outside,
            };
            continue;
        }
        let Fence::Shell(cwd) = &mut fence else {
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
                ["cd", target] if is_literal_repo_path(target) => {
                    dir.push(target);
                    commands.push(ShellCommand {
                        line: index + 1,
                        command,
                        kind: CommandKind::ChangeDir { dir: dir.clone() },
                    });
                }
                ["cd", ..] => *cwd = None,
                [runner, "run", script, ..] if RUNNERS.contains(runner) && is_literal(script) => {
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

fn is_literal(word: &str) -> bool {
    !word.is_empty() && !word.contains(['$', '*', '?', '~', '<', '>', '`', '\'', '"', '\\'])
}

fn is_literal_repo_path(path: &str) -> bool {
    is_literal(path)
        && !path.starts_with('/')
        && !path.starts_with(".ivar")
        && !path.split('/').any(|component| component == "..")
}

#[cfg(test)]
#[path = "../../tests/unit/domain/plan_commands.rs"]
mod tests;
