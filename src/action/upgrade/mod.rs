//! The update notice and `ivar upgrade`. The one place `ivar` makes a network
//! call nobody asked for; see ARCHITECTURE.md.

pub mod cache;
pub mod command;
pub mod notice;

pub use notice::Notice;
