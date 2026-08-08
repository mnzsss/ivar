//! `ivar`'s TUI: a sidebar of repos (or features) with a live shell panel.
//!
//! # The four rules that carry this module
//!
//! - **`widget` is pure.** It never awaits, never opens anything, never reads
//!   the clock. Snapshot in, `Buffer` out. Tested against ratatui's
//!   `TestBackend` headlessly.
//! - **`driver` owns every byte of I/O.** PTY reads/writes, spawn, resize.
//!   Exposes explicit step methods the host loop calls; spawns shells lazily
//!   (one per promoted repo, on first focus), never background tasks.
//! - **`key_router` is a pure reducer.** `(mode, key) -> (mode, action)`. The
//!   only place a keystroke becomes intent.
//! - **`screen` is the emulator seam.** PTY bytes in, text viewport out. The
//!   `vt100` swap point.
//!
//! `master_detail` is the host loop: it initialises the terminal, owns the
//! event loop (keys in, pump, render), and restores the terminal on the way
//! out. The action layer bridges the hall's real state into the driver by
//! pushing in ready-made [`ShellSpec`](driver::ShellSpec)s and
//! [`Row`](widget::Row)s — the TUI never reads the hall itself.

pub mod driver;
pub mod key_router;
pub mod master_detail;
pub mod screen;
pub mod widget;
