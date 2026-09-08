//! Graph infrastructure modules: SQLite database layer and schema migrations.

pub mod db;
pub mod schema;

pub use db::{FileRow, GraphDb, GraphDbError, RepoRow};
