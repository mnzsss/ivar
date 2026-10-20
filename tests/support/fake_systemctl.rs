#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use camino::{Utf8Path, Utf8PathBuf};

const FAKE_SYSTEMCTL: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$SYSTEMCTL_FAKE_LOG"
case "$2" in
  show) [ -n "$SYSTEMCTL_FAKE_NEXT" ] && echo "$SYSTEMCTL_FAKE_NEXT" ;;
  show-environment)
    [ -n "$SYSTEMCTL_FAKE_DOWN" ] && exit 1
    [ -n "$SYSTEMCTL_FAKE_PATH" ] && printf 'HOME=/h\nPATH=%s\n' "$SYSTEMCTL_FAKE_PATH" ;;
esac
exit 0
"#;

pub(crate) struct FakeSystemctl {
    pub(crate) dir: Utf8PathBuf,
    pub(crate) log: Utf8PathBuf,
}

impl FakeSystemctl {
    pub(crate) fn install(root: &Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-bin-systemctl");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("systemctl");
        std::fs::write(&script, FAKE_SYSTEMCTL).unwrap();
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
