//! Session state on disk: `state.json` inside each session's View Dir.
//!
//! One file per session, written through the versioned [`Store`] with
//! [`Policy::Local`] — session state is derived local state, so a read that
//! migrates persists the migrated form, silently.
//!
//! The file lives *inside* the View Dir on purpose: `session convert` moves
//! the View Dir from `.ivar/sessions/<id>/` to
//! `.ivar/features/<feature>/sessions/<id>/`, and the state moves with it —
//! preserving `provider` and `started_at` is then just not deleting anything.

use camino::Utf8Path;

use crate::domain::session::SessionState;
use crate::error::Failure;
use crate::store::versioned::{Policy, Store};

/// `state.json`'s schema version. Matches [`SessionState`]'s own constant —
/// the type owns the number, this module just wires it into the store.
const CURRENT_VERSION: u32 = 1;

/// The filename every session's record lives in, inside its View Dir.
const STATE_FILE: &str = "state.json";

impl SessionState {
    /// Read `state.json` from the session's View Dir. `Ok(None)` when the
    /// session predates state files, or was never given one.
    pub fn read(view_dir: &Utf8Path) -> Result<Option<Self>, Failure> {
        store(view_dir).read().map_err(Failure::from)
    }

    /// Write this session's record to `state.json` inside `view_dir`,
    /// atomically, in canonical form. The View Dir already exists — sessions
    /// are materialised before their state is written.
    pub fn write(&self, view_dir: &Utf8Path) -> Result<(), Failure> {
        store(view_dir).write(self).map_err(Failure::from)
    }
}

/// The versioned store over one session's file.
fn store(view_dir: &Utf8Path) -> Store<SessionState> {
    Store::new(
        view_dir.join(STATE_FILE),
        Vec::new(),
        CURRENT_VERSION,
        Policy::Local,
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::domain::name::FeatureName;
    use crate::domain::provider::Provider;
    use crate::infra::fs;
    use crate::test_support::utf8_temp_dir;

    #[test]
    fn absent_state_reads_as_ok_none() {
        let (_dir, root) = utf8_temp_dir();
        let view_dir = root
            .join("sessions")
            .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
        fs::ensure_dir(&view_dir).unwrap();

        assert_eq!(SessionState::read(&view_dir).unwrap(), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let (_dir, root) = utf8_temp_dir();
        let view_dir = root
            .join("sessions")
            .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
        fs::ensure_dir(&view_dir).unwrap();
        let mut state = SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00.000000000Z");
        state.bind(
            FeatureName::new("checkout").unwrap(),
            "2026-01-02T00:00:00.000000000Z",
        );

        state.write(&view_dir).unwrap();
        let read_back = SessionState::read(&view_dir).unwrap().unwrap();

        assert_eq!(read_back, state);
    }

    #[test]
    fn the_file_is_written_inside_the_view_dir_with_a_version_stamp() {
        let (_dir, root) = utf8_temp_dir();
        let view_dir = root
            .join("sessions")
            .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
        fs::ensure_dir(&view_dir).unwrap();
        let state = SessionState::new(Provider::OpenCode, "2026-01-01T00:00:00.000000000Z");

        state.write(&view_dir).unwrap();

        let text = fs::read_text(&view_dir.join("state.json"))
            .unwrap()
            .unwrap();
        assert!(text.contains("\"version\": 1"), "was: {text}");
        assert!(text.contains("\"provider\": \"opencode\""), "was: {text}");
        assert!(text.contains("\"feature\": null"), "was: {text}");
    }
}
