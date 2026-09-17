#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn change_dir(line: usize, command: &str, dir: &str) -> ShellCommand {
    ShellCommand {
        line,
        block: 0,
        command: command.to_owned(),
        kind: CommandKind::ChangeDir {
            dir: Utf8PathBuf::from(dir),
        },
    }
}

fn run_script(line: usize, command: &str, dir: &str, script: &str) -> ShellCommand {
    ShellCommand {
        line,
        block: 0,
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
        scan_shell_commands(source, &[]),
        vec![
            change_dir(3, "cd packages/web", "packages/web"),
            run_script(4, "pnpm run build", "packages/web", "build"),
        ]
    );
}

#[test]
fn chained_commands_split_and_each_block_starts_at_the_repo_root() {
    let source = "```sh\ncd api && npm run lint\n```\n```console\n$ pnpm run test\n```\n";

    let mut expected = vec![
        change_dir(2, "cd api", "api"),
        run_script(2, "npm run lint", "api", "lint"),
        run_script(5, "pnpm run test", "", "test"),
    ];
    expected[2].block = 1;

    assert_eq!(scan_shell_commands(source, &[]), expected);
}

#[test]
fn commands_outside_shell_fences_are_ignored() {
    let source = "cd nowhere\n```rust\ncd nowhere\n```\n```text\nnpm run x\n```\n";

    assert!(scan_shell_commands(source, &[]).is_empty());
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
            scan_shell_commands(&source, &[]).is_empty(),
            "`cd {target}` must not be reported nor let the run through"
        );
    }
}

#[test]
fn a_script_that_is_not_literal_is_skipped() {
    let source = "```sh\nnpm run $TARGET\nnpm test\n```\n";

    assert!(scan_shell_commands(source, &[]).is_empty());
}

#[test]
fn a_run_with_flags_or_a_workspace_scope_is_skipped() {
    for command in [
        "npm run --silent build",
        "pnpm run -r build",
        "pnpm --filter web run build",
        "pnpm run build --filter web",
        "pnpm -C packages/web run build",
        "npm run build -w web",
        "npm run build --workspace=web",
        "npm --prefix web run build",
        "pnpm run --recursive build",
    ] {
        let source = format!("```sh\n{command}\n```\n");
        assert!(
            scan_shell_commands(&source, &[]).is_empty(),
            "`{command}` must be skipped"
        );
    }
}

#[test]
fn yarn_runs_are_never_reported() {
    let source = "```sh\nyarn run build\nyarn build\n```\n";

    assert!(scan_shell_commands(source, &[]).is_empty());
}

#[test]
fn cd_dash_stops_tracking_the_block() {
    let source = "```sh\ncd -\nnpm run test\n```\n";

    assert!(scan_shell_commands(source, &[]).is_empty());
}

#[test]
fn a_dot_slash_prefixed_ivar_path_stops_tracking_the_block() {
    let source = "```sh\ncd ./.ivar/repos/api\nnpm run test\n```\n";

    assert!(scan_shell_commands(source, &[]).is_empty());
}

#[test]
fn a_cd_into_a_declared_repo_name_stops_tracking_the_block() {
    let source = "```sh\ncd api/packages\nnpm run test\n```\n```sh\ncd ./api\nnpm run test\n```\n";

    assert!(scan_shell_commands(source, &["api".to_owned()]).is_empty());
}

#[test]
fn tilde_fences_open_and_close_shell_blocks() {
    let source = "~~~bash\ncd web\n~~~\ncd outside\n";

    assert_eq!(
        scan_shell_commands(source, &[]),
        vec![change_dir(2, "cd web", "web")]
    );
}

#[test]
fn a_fence_closes_only_on_the_same_marker_at_least_as_long() {
    let source = "````sh\n```\ncd inner\n~~~~\ncd still-inner\n````\ncd outside\n";

    assert_eq!(
        scan_shell_commands(source, &[]),
        vec![
            change_dir(3, "cd inner", "inner"),
            change_dir(5, "cd still-inner", "inner/still-inner"),
        ]
    );
}

#[test]
fn a_mkdir_is_reported_relative_to_the_current_directory() {
    let source = "```sh\ncd packages\nmkdir -p web/src\n```\n";

    assert_eq!(
        scan_shell_commands(source, &[])[1].kind,
        CommandKind::MakeDir {
            dir: Utf8PathBuf::from("packages/web/src"),
        }
    );
}

#[test]
fn each_fenced_shell_block_is_numbered() {
    let source = "```sh\ncd a\n```\ntext\n```sh\ncd b\n```\n";

    let blocks: Vec<usize> = scan_shell_commands(source, &[])
        .iter()
        .map(|command| command.block)
        .collect();
    assert_eq!(blocks, vec![0, 1]);
}
