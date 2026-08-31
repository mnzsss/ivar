//! Unit tests for `crate::infra::proc`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::error::Status;
use crate::test_support::utf8_temp_dir;

// -- capture: the exit code is data ---------------------------------------

#[test]
fn capture_returns_stdout_and_a_zero_code_on_success() {
    let output = capture(&Command::new("echo").arg("hello")).unwrap();

    assert!(output.success());
    assert_eq!(output.code, Some(0));
    assert_eq!(output.stdout, "hello");
    assert_eq!(output.stderr, "");
}

/// If a non-zero exit were an `Err`, every caller that cares about *which*
/// non-zero code it got would have to dig it back out of an error type.
#[test]
fn a_non_zero_exit_is_ok_not_err() {
    let output = capture(&Command::new("sh").arg("-c").arg("exit 3")).unwrap();

    assert!(!output.success());
    assert_eq!(output.code, Some(3));
}

#[test]
fn capture_separates_the_two_streams() {
    let output = capture(
        &Command::new("sh")
            .arg("-c")
            .arg("printf out; printf err >&2"),
    )
    .unwrap();

    assert_eq!(output.stdout, "out");
    assert_eq!(output.stderr, "err");
}

#[test]
fn captured_output_has_its_trailing_newline_trimmed() {
    let output = capture(&Command::new("sh").arg("-c").arg("printf 'line\\n\\n'")).unwrap();

    assert_eq!(output.stdout, "line");
}

#[test]
fn invalid_utf8_is_decoded_lossily_rather_than_failing() {
    let output = capture(&Command::new("sh").arg("-c").arg("printf 'a\\377b'")).unwrap();

    assert!(output.success());
    assert!(
        output.stdout.contains('\u{fffd}'),
        "expected a replacement character, got {:?}",
        output.stdout
    );
}

// -- cwd and env ----------------------------------------------------------

#[test]
fn cwd_is_where_the_program_runs() {
    let (_guard, dir) = utf8_temp_dir();
    let canonical = dir.canonicalize_utf8().unwrap();

    let output = capture(&Command::new("pwd").cwd(&canonical)).unwrap();

    assert_eq!(output.stdout, canonical.as_str());
}

#[test]
fn env_overrides_are_visible_to_the_program() {
    let output = capture(
        &Command::new("sh")
            .arg("-c")
            .arg("printf %s \"$IVAR_TEST_VALUE\"")
            .env("IVAR_TEST_VALUE", "set-by-the-test"),
    )
    .unwrap();

    assert_eq!(output.stdout, "set-by-the-test");
}

/// Setting a variable to the empty string is how git is told to stop
/// looking for an askpass helper. It must reach the child as *set and
/// empty*, not as absent — the two mean different things to git.
#[test]
fn an_empty_env_value_is_set_rather_than_unset() {
    let output = capture(
        &Command::new("sh")
            .arg("-c")
            .arg("if [ -n \"${IVAR_TEST_EMPTY+x}\" ]; then printf set; else printf unset; fi")
            .env("IVAR_TEST_EMPTY", ""),
    )
    .unwrap();

    assert_eq!(output.stdout, "set");
}

// -- the one error --------------------------------------------------------

#[test]
fn a_program_that_does_not_exist_is_a_spawn_error() {
    let error =
        capture(&Command::new("ivar-no-such-program-exists-anywhere")).expect_err("no binary");

    assert!(matches!(error, Error::Spawn { .. }));
}

#[test]
fn spawn_failure_converts_to_a_blocked_failure_naming_the_program() {
    let error = capture(&Command::new("ivar-no-such-program-exists-anywhere").arg("--version"))
        .expect_err("no binary");

    let failure: Failure = error.into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "proc.spawn_failed");
    assert!(
        failure
            .what
            .contains("ivar-no-such-program-exists-anywhere"),
        "message was: {}",
        failure.what
    );
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn a_missing_working_directory_is_a_spawn_error_too() {
    let (_guard, dir) = utf8_temp_dir();

    let error = capture(&Command::new("echo").cwd(dir.join("does-not-exist")))
        .expect_err("cwd does not exist");

    assert!(matches!(error, Error::Spawn { .. }));
}

// -- inherit --------------------------------------------------------------

#[test]
fn inherit_returns_the_exit_code() {
    assert_eq!(inherit(&Command::new("true")).unwrap(), Some(0));
    assert_eq!(
        inherit(&Command::new("sh").arg("-c").arg("exit 7")).unwrap(),
        Some(7)
    );
}

// -- stream: the whole point ------------------------------------------------

/// This is the operation. A test that only reads lines after the child has
/// exited would pass against [`capture`] too and prove nothing about
/// [`stream`] existing at all.
#[test]
fn stream_yields_a_line_while_the_child_is_still_running() {
    let mut child_stream = stream(
        &Command::new("sh")
            .arg("-c")
            .arg("echo first; sleep 0.3; echo second"),
    )
    .unwrap();

    let first = child_stream.read_line().unwrap();
    assert_eq!(first.as_deref(), Some("first"));

    // Read while the `sleep 0.3` is still running, not after the process
    // already exited — `try_wait` returning `Ok(None)` is "still alive".
    assert!(
        matches!(child_stream.child.try_wait(), Ok(None)),
        "the child had already exited by the time the first line was read"
    );

    let second = child_stream.read_line().unwrap();
    assert_eq!(second.as_deref(), Some("second"));

    assert_eq!(child_stream.read_line().unwrap(), None);
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

#[test]
fn stream_reads_multiple_lines_in_order_and_then_ends() {
    let mut child_stream =
        stream(&Command::new("sh").arg("-c").arg("printf 'a\\nb\\nc\\n'")).unwrap();

    assert_eq!(child_stream.read_line().unwrap().as_deref(), Some("a"));
    assert_eq!(child_stream.read_line().unwrap().as_deref(), Some("b"));
    assert_eq!(child_stream.read_line().unwrap().as_deref(), Some("c"));
    assert_eq!(child_stream.read_line().unwrap(), None);
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

/// A child that writes more to stderr than the pipe buffer holds, before it
/// closes stdout, must not deadlock the reader.
///
/// The shape that hangs: `read_line` blocks waiting for stdout, the child
/// blocks writing a full stderr pipe, and `wait` — the only thing that drains
/// stderr — is never reached because the caller is still reading. A provider
/// logging steadily to stderr during a long session is exactly this, and the
/// symptom is a session command hanging with no output, indistinguishable from
/// a slow provider.
///
/// Run on a worker with a timeout rather than inline: a deadlocked assertion
/// would hang the whole test binary instead of failing it.
#[test]
fn stream_does_not_deadlock_when_the_child_floods_stderr() {
    // 256 KiB, four times the usual 64 KiB pipe buffer, all written before
    // the single stdout line the reader is waiting for.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut child_stream = stream(
            &Command::new("sh")
                .arg("-c")
                .arg("yes stderr-noise | head -c 262144 >&2; echo done"),
        )
        .unwrap();
        let line = child_stream.read_line().unwrap();
        let end = child_stream.read_line().unwrap();
        let code = child_stream.wait().unwrap();
        let _ = tx.send((line, end, code, child_stream.stderr().len()));
    });

    let Ok((line, end, code, stderr_len)) = rx.recv_timeout(std::time::Duration::from_secs(10))
    else {
        panic!(
            "stream deadlocked: the child filled its stderr pipe before closing \
             stdout, so read_line waited for a line the child was blocked from \
             writing. stderr must be drained while stdout is being read."
        );
    };

    assert_eq!(line.as_deref(), Some("done"));
    assert_eq!(end, None);
    assert_eq!(code, Some(0));
    // Trimmed by `decode`, so assert the bulk arrived rather than an exact size.
    assert!(
        stderr_len > 260_000,
        "stderr was truncated: got {stderr_len} bytes of the 262144 written"
    );
}

#[test]
fn stream_wait_returns_the_exit_code_like_inherit() {
    let mut child_stream = stream(&Command::new("sh").arg("-c").arg("exit 7")).unwrap();

    while child_stream.read_line().unwrap().is_some() {}

    assert_eq!(child_stream.wait().unwrap(), Some(7));
}

#[test]
fn stream_captures_stderr_for_explaining_a_failure() {
    let mut child_stream = stream(
        &Command::new("sh")
            .arg("-c")
            .arg("printf trouble >&2; exit 1"),
    )
    .unwrap();

    while child_stream.read_line().unwrap().is_some() {}
    assert_eq!(child_stream.wait().unwrap(), Some(1));

    assert_eq!(child_stream.stderr(), "trouble");
}

/// Mirrors `an_empty_env_value_is_set_rather_than_unset`'s sibling
/// concern for `capture`: a process nobody can see must never block
/// waiting on a prompt. Still the default — only a caller that supplied text
/// with `Command::stdin` gets a pipe.
#[test]
fn stream_stdin_is_null_so_a_read_does_not_block() {
    let mut child_stream =
        stream(&Command::new("sh").arg("-c").arg("read -r line; echo done")).unwrap();

    assert_eq!(child_stream.read_line().unwrap().as_deref(), Some("done"));
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

// -- stream: feeding the child --------------------------------------------

#[test]
fn supplied_stdin_reaches_the_child_verbatim() {
    let mut child_stream = stream(&Command::new("cat").stdin("hello ivar")).unwrap();

    assert_eq!(
        child_stream.read_line().unwrap().as_deref(),
        Some("hello ivar")
    );
    assert_eq!(child_stream.read_line().unwrap(), None);
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

/// The reason the prompt travels this way at all: text that argv would mangle
/// or mistake for a flag arrives byte for byte, newlines and quotes included.
#[test]
fn supplied_stdin_carries_newlines_quotes_and_leading_dashes_untouched() {
    let mut child_stream =
        stream(&Command::new("cat").stdin("--not-a-flag\nwith \"quotes\" and  spaces\n")).unwrap();

    assert_eq!(
        child_stream.read_line().unwrap().as_deref(),
        Some("--not-a-flag")
    );
    assert_eq!(
        child_stream.read_line().unwrap().as_deref(),
        Some("with \"quotes\" and  spaces")
    );
    assert_eq!(child_stream.read_line().unwrap(), None);
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

/// The child must see EOF, not a pipe held open: `cat` only exits once its
/// input ends, and `wc -c` only answers once it has counted everything.
#[test]
fn supplied_stdin_is_closed_so_the_child_sees_eof() {
    let big = "x".repeat(512 * 1024);
    let mut child_stream = stream(&Command::new("wc").arg("-c").stdin(big)).unwrap();

    let counted: usize = child_stream
        .read_line()
        .unwrap()
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    assert_eq!(counted, 512 * 1024);
    assert_eq!(child_stream.wait().unwrap(), Some(0));
}

/// A child that exits without reading breaks the writer's pipe. That is the
/// child's own exit code to explain, so the write is dropped rather than
/// reported as a second, competing failure.
#[test]
fn a_child_that_never_reads_its_stdin_is_not_a_write_failure() {
    let mut child_stream = stream(
        &Command::new("sh")
            .arg("-c")
            .arg("echo ignored; exit 3")
            .stdin("x".repeat(512 * 1024)),
    )
    .unwrap();

    assert_eq!(
        child_stream.read_line().unwrap().as_deref(),
        Some("ignored")
    );
    assert_eq!(child_stream.wait().unwrap(), Some(3));
}

#[test]
fn stdin_text_is_absent_unless_a_caller_supplies_it() {
    assert_eq!(Command::new("cat").stdin_text(), None);
    assert_eq!(Command::new("cat").stdin("in").stdin_text(), Some("in"));
}

/// `display` names the invocation for a human reading a failure. Supplied
/// stdin is input, often enormous, and belongs nowhere near that line.
#[test]
fn supplied_stdin_never_shows_up_in_display() {
    let display = Command::new("cat").stdin("a very long prompt").display();

    assert_eq!(display, "cat");
}

#[test]
fn stream_reports_a_spawn_error_like_capture_and_inherit() {
    let error =
        stream(&Command::new("ivar-no-such-program-exists-anywhere")).expect_err("no binary");

    assert!(matches!(error, Error::Spawn { .. }));

    let failure: Failure = error.into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "proc.spawn_failed");
}

// -- display, diagnostic, availability ------------------------------------

#[test]
fn display_renders_the_invocation_a_human_reads_back() {
    let command = Command::new("git").arg("clone").arg("--bare").arg("url");

    assert_eq!(command.display(), "git clone --bare url");
}

#[test]
fn diagnostic_prefers_stderr_then_stdout_then_the_exit_itself() {
    let with_stderr = Output {
        code: Some(1),
        stdout: "out".to_owned(),
        stderr: "err".to_owned(),
    };
    assert_eq!(with_stderr.diagnostic(), "err");

    let stdout_only = Output {
        code: Some(1),
        stdout: "out".to_owned(),
        stderr: String::new(),
    };
    assert_eq!(stdout_only.diagnostic(), "out");

    let silent = Output {
        code: Some(1),
        stdout: String::new(),
        stderr: String::new(),
    };
    assert_eq!(silent.diagnostic(), "exited 1 with no output");

    let signalled = Output {
        code: None,
        stdout: String::new(),
        stderr: String::new(),
    };
    assert!(!signalled.success());
    assert_eq!(signalled.diagnostic(), "killed by a signal");
}

// -- working-directory attribution via /proc ------------------------------

/// The occupancy signal behind `session connect --create`: this test process
/// is running in its own working directory, so its own program name must be
/// found there.
#[test]
fn is_program_running_in_finds_this_process_in_its_own_cwd() {
    let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap())
        .expect("the test's working directory is UTF-8");
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "ivar".to_owned());

    assert!(is_program_running_in(&cwd, &program));
}

/// A directory nothing runs in holds nobody — this is what `--create` reads as
/// "free", and reading it wrong would silently duplicate a live session.
#[test]
fn is_program_running_in_is_false_for_a_directory_with_no_processes() {
    let dir = camino::Utf8PathBuf::from("/no-such-view-dir-xyz-12345");
    assert!(!is_program_running_in(&dir, "node"));
}

/// The program filter is load-bearing: `ivar` and the shell that ran it are
/// cwd'd inside a View Dir, so an unfiltered walk would call every session
/// busy — including the caller's own.
///
/// The needle is built at run time on purpose. A literal would be a substring
/// of this test file's path, and of whatever shell command line invoked the
/// run — both of which are cmdlines the walk reads, in this very directory.
#[test]
fn is_program_running_in_is_false_for_a_program_that_is_not_there() {
    let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap())
        .expect("the test's working directory is UTF-8");
    let needle = format!(
        "absent-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    assert!(!is_program_running_in(&cwd, &needle));
}

// -- cwd and PWD are set together -----------------------------------------

/// Reads back one variable from the child's own environment. `env` is used
/// rather than a shell because a shell repairs a stale `PWD` at startup,
/// which is exactly the lie under test.
fn child_env(command: &Command, key: &str) -> Option<String> {
    let output = capture(command).unwrap();
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(ToOwned::to_owned)
}

/// `current_dir` does not touch `PWD`, so a child spawned by this crate used
/// to inherit *this* process's `PWD` and read back a directory it was not in.
/// An OpenCode executor did exactly that: spawned in a Feature Session View
/// Dir, it took `$PWD` — the hall — as its project directory and ran the
/// whole session there.
#[test]
fn a_child_reads_back_the_working_directory_it_was_given() {
    let (_guard, dir) = utf8_temp_dir();

    let pwd = child_env(&Command::new("env").cwd(dir.clone()), "PWD");

    assert_eq!(pwd.as_deref(), Some(dir.as_str()));
}

/// The pairing is not a second guess at the caller's intent: a caller that
/// sets `PWD` itself still wins, because the overrides are applied after.
#[test]
fn an_explicit_pwd_override_still_wins_over_the_working_directory() {
    let (_guard, dir) = utf8_temp_dir();

    let pwd = child_env(
        &Command::new("env").cwd(dir).env("PWD", "/somewhere/else"),
        "PWD",
    );

    assert_eq!(pwd.as_deref(), Some("/somewhere/else"));
}

/// A command with no working directory of its own says nothing about `PWD`
/// either — it inherits the parent's, which is the whole environment it
/// inherits.
#[test]
fn a_command_without_a_working_directory_does_not_invent_a_pwd() {
    let inherited = std::env::var("PWD").ok();

    let pwd = child_env(&Command::new("env"), "PWD");

    assert_eq!(pwd, inherited);
}
