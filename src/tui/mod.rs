//! `ivar session`'s TUI: a master-detail view with an embedded agent PTY.
//!
//! # The four rules that carry this module
//!
//! - **`widget` is pure.** It never awaits, never opens anything, never reads
//!   the clock. Snapshot in, `Buffer` out. Tested against ratatui's
//!   `TestBackend` headlessly.
//! - **`driver` owns every byte of I/O.** PTY reads/writes, crossterm events,
//!   resize. Exposes explicit step methods the host loop calls; spawns no
//!   background tasks.
//! - **`key_router` is a pure reducer.** `(mode, key) -> (mode, action)`. The
//!   only place a keystroke becomes intent.
//! - **`screen` is the emulator seam.** PTY bytes in, text viewport out. The
//!   `vt100` swap point.
//!
//! `master_detail` bridges the hall's real state (features, repos, their
//! worktree states) into the widget's [`Snapshot`] — it is where "what the
//! hall looks like" becomes "what the TUI shows".

pub mod driver;
pub mod key_router;
pub mod master_detail;
pub mod screen;
pub mod widget;
