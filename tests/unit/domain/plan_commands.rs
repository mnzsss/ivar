#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn change_dir(line: usize, command: &str, dir: &str) -> ShellCommand {
    ShellCommand {
        line,
        command: command.to_owned(),
        kind: CommandKind::ChangeDir {
            dir: Utf8PathBuf::from(dir),
        },
    }
}

fn run_script(line: usize, command: &str, dir: &str, script: &str) -> ShellCommand {
    ShellCommand {
        line,
        command: command.to_owned(),
        kind: CommandKind::RunScript {
            dir: Utf8PathBuf::from(dir),
            script: script.to_owned(),
        },
    }
}

#[test]
fn a_run_resolves_against_the_cd_before_it() {
    let source = "intro\n```bash\ncd packages/web\npnpm run build\n```\n";

    assert_eq!(
        scan_shell_commands(source),
        vec![
            change_dir(3, "cd packages/web", "packages/web"),
            run_script(4, "pnpm run build", "packages/web", "build"),
        ]
    );
}

#[test]
fn chained_commands_split_and_each_block_starts_at_the_repo_root() {
    let source = "```sh\ncd api && npm run lint\n```\n```console\n$ yarn run test\n```\n";

    assert_eq!(
        scan_shell_commands(source),
        vec![
            change_dir(2, "cd api", "api"),
            run_script(2, "npm run lint", "api", "lint"),
            run_script(5, "yarn run test", "", "test"),
        ]
    );
}

#[test]
fn commands_outside_shell_fences_are_ignored() {
    let source = "cd nowhere\n```rust\ncd nowhere\n```\n```text\nnpm run x\n```\n";

    assert!(scan_shell_commands(source).is_empty());
}

#[test]
fn a_cd_that_is_not_literal_stops_tracking_the_block() {
    for target in [
        "$REPO",
        "~/code",
        "../sibling",
        "/abs",
        ".ivar/repos/api/checkout",
        "<repo>",
        "",
    ] {
        let source = format!("```sh\ncd {target}\nnpm run test\n```\n");
        assert!(
            scan_shell_commands(&source).is_empty(),
            "`cd {target}` must not be reported nor let the run through"
        );
    }
}

#[test]
fn a_script_that_is_not_literal_is_skipped() {
    let source = "```sh\nnpm run $TARGET\nnpm test\n```\n";

    assert!(scan_shell_commands(source).is_empty());
}
