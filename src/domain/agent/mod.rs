//! Scheduled agents: what a valid agent file is, how its schedule maps to
//! systemd, and what a run records. No I/O.

pub mod container;
pub mod cron;
pub mod images;
pub mod receipt;
pub mod spec;
pub mod tool;
pub mod unit;
