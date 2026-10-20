#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use camino::{Utf8Path, Utf8PathBuf};

const FAKE_DOCKER: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_FAKE_LOG"
case "$1" in
  run)
    case " $* " in *" -d "*) exit 0 ;; esac
    [ -n "$DOCKER_FAKE_OUTPUT" ] && printf '%s\n' "$DOCKER_FAKE_OUTPUT"
    [ -n "$DOCKER_FAKE_WRITE" ] && printf 'agent edit\n' >> "$DOCKER_FAKE_WRITE"
    [ -n "$DOCKER_FAKE_PLANT" ] && printf 'gitdir: %s\n' "$DOCKER_FAKE_PLANT_TARGET" > "$DOCKER_FAKE_PLANT"
    [ -n "$DOCKER_FAKE_COPY_ENV_FILE" ] && cp "$(printf '%s\n' "$@" | sed -n '/^--env-file$/{n;p;}')" "$DOCKER_FAKE_COPY_ENV_FILE"
    [ -n "$DOCKER_FAKE_SLEEP" ] && sleep "$DOCKER_FAKE_SLEEP"
    exit "${DOCKER_FAKE_EXIT:-0}" ;;
  info)
    [ -n "$DOCKER_FAKE_DOWN" ] && exit 1
    case " $* " in *SecurityOptions*) echo "${DOCKER_FAKE_SECURITY:-[\"name=seccomp\"]}" ;; *) echo 28.0.0 ;; esac ;;
  *) exit 0 ;;
esac
"#;

pub(crate) struct FakeDocker {
    pub(crate) dir: Utf8PathBuf,
    pub(crate) log: Utf8PathBuf,
}

impl FakeDocker {
    pub(crate) fn install(root: &Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-bin-docker");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("docker");
        std::fs::write(&script, FAKE_DOCKER).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let log = dir.join("log");
        std::fs::write(&log, "").unwrap();
        Self { dir, log }
    }

    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }
}
