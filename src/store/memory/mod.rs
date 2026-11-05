//! The hall memory database, `.ivar/memory.db`, and the one-time split that
//! moved the code graph out of that path into `.ivar/graph.db`.

pub mod db;
pub mod observations;
pub mod retention;
pub mod schema;
pub mod split;
pub mod spool;
pub mod page_source;
