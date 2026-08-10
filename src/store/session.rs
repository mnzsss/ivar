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
#[path = "../../tests/unit/store/session.rs"]
mod tests;
