//! Outcomes and human-readable output formatting for graph subcommands.

pub mod ops;
pub mod query;

pub use ops::{
    AffectedOutcome, CleanOutcome, ExploreOutcome, GraphViewOutcome, IndexBatchOutcome, McpOutcome,
    McpRegistration, PathOutcome, VizOutcome,
};
pub use query::{
    CalleesOutcome, CallersOutcome, ComplexityOutcome, DeadCodeOutcome, FileOutcome, FindOutcome,
    HierarchyOutcome, ImpactOutcome, MissesOutcome, StatsOutcome,
};

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/index_outcome.rs"]
mod index_outcome_tests;
