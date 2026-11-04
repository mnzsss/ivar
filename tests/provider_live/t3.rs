use std::process::Command;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::common::hall_root;
use crate::support::run_ok;

/// `ivar <args> --json` with the developer's real HOME, so the `t3` it spawns finds the
/// running server, and with no inherited ivar session.
pub(crate) fn real_ivar(root: &Utf8Path, args: &[&str]) -> Value {
    let mut command = crate::common::ivar();
    for key in [
        "IVAR_SESSION_ID",
        "IVAR_SESSION_PATH",
        "IVAR_FEATURE",
        "IVAR_HALL",
        "IVAR_PROVIDER",
    ] {
        command.env_remove(key);
    }
    let output = command
        .env("IVAR_NO_UPDATE_CHECK", "1")
        .current_dir(root)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`ivar {}` printed no JSON ({error}): {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub(crate) fn t3_program() -> String {
    std::env::var("T3CODE_CLI_PATH")
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "t3".to_owned())
}

pub(crate) fn t3_command(args: &[&str]) -> std::process::Output {
    Command::new(t3_program()).args(args).output().unwrap()
}

pub(crate) fn wait_for_t3_project(view_dir: &Utf8PathBuf) {
    for _ in 0..15 {
        if t3_command(&["project", "rename", view_dir.as_str(), "ivar-live"])
            .status
            .success()
        {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    panic!("T3 did not register {view_dir} within 15 seconds");
}

pub(crate) fn t3_hall() -> (tempfile::TempDir, Utf8PathBuf, String, Utf8PathBuf) {
    let (guard, root) = hall_root();
    run_ok(&root, &["init"]);
    run_ok(&root, &["provider", "add", "t3"]);
    run_ok(&root, &["sync"]);
    run_ok(&root, &["feature", "create", "t3-live"]);
    let started = real_ivar(&root, &["session", "start", "t3-live", "--provider", "t3"]);
    assert_eq!(started["ok"], true, "{started}");
    let id = started["session_id"].as_str().unwrap().to_owned();
    let view_dir = Utf8PathBuf::from(started["view_dir"].as_str().unwrap());
    (guard, root, id, view_dir)
}
