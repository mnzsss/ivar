//! Codebase graph persistence and AST extraction layer.

pub mod db;
pub mod extractor;
pub mod schema;

pub use db::{FileRow, GraphDb, GraphDbError, RepoRow};
pub use extractor::{ExtractedFile, ExtractorError, extract_file};
