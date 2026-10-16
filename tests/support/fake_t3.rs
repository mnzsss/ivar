#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use camino::{Utf8Path, Utf8PathBuf};

const FAKE_T3: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$T3_FAKE_LOG"
if [ "$1" = "--version" ]; then echo "0.1.0"; exit 0; fi
if [ -n "$T3_FAKE_FAIL" ]; then
  printf 'DesktopAppRequestFailedError: fake failure\n    at fake (fake.mjs:1:1)\n' >&2
  exit 1
fi
case "$1" in
  app)
    if [ ! -d "$2" ]; then
      printf 'DesktopAppRequestFailedError: T3 Code could not open %s (project-create-failed).\n' "$2" >&2
      exit 1
    fi
    grep -qxF "$2" "$T3_FAKE_PROJECTS" || printf '%s\n' "$2" >> "$T3_FAKE_PROJECTS"
    printf 'Opened %s in T3 Code.\n' "$2"
    ;;
  project)
    [ "$2" = remove ] || exit 2
    if ! grep -qxF "$3" "$T3_FAKE_PROJECTS"; then
      printf "ProjectNotFoundError: No active project found for '%s'.\n" "$3" >&2
      exit 1
    fi
    case " $* " in
      *" --force "*) ;;
      *) if [ -n "$T3_FAKE_THREADS" ]; then
           printf 'ProjectLiveServerDeclaredResponseError: Server request failed (invalid_request, trace 00000000000000000000000000000000).\n' >&2
           exit 1
         fi ;;
    esac
    grep -vxF "$3" "$T3_FAKE_PROJECTS" > "$T3_FAKE_PROJECTS.tmp"
    mv "$T3_FAKE_PROJECTS.tmp" "$T3_FAKE_PROJECTS"
    printf 'Removed project fake-id (fake).\n'
    ;;
  *) exit 2 ;;
esac
"#;

pub(crate) struct FakeT3 {
    pub(crate) dir: Utf8PathBuf,
    pub(crate) projects: Utf8PathBuf,
    pub(crate) log: Utf8PathBuf,
}

impl FakeT3 {
    pub(crate) fn install(root: &Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-bin-t3");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("t3");
        std::fs::write(&script, FAKE_T3).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let projects = dir.join("projects");
        let log = dir.join("log");
        std::fs::write(&projects, "").unwrap();
        std::fs::write(&log, "").unwrap();
        Self { dir, projects, log }
    }

    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }

    pub(crate) fn projects(&self) -> Vec<String> {
        std::fs::read_to_string(&self.projects)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
