use std::path::Path;

use crate::action::Ctx;
use crate::action::discover_hall;
use crate::action::graph::outcome::{
    AffectedOutcome, CalleesOutcome, CallersOutcome, ComplexityOutcome, DeadCodeOutcome,
    ExploreOutcome, FileOutcome, FindOutcome, HierarchyOutcome, ImpactOutcome, MissesOutcome,
    PathOutcome,
};
use crate::domain::graph::UsageEvent;
use crate::store::graph::db::GraphDb;

pub trait ResultCount {
    fn result_count(&self) -> Option<usize>;
}

pub fn record_usage(ctx: &Ctx, event: &UsageEvent) {
    let Ok(layout) = discover_hall(ctx) else {
        return;
    };
    record_usage_at(layout.ivar_dir().join("memory.db").as_std_path(), event);
}

pub fn record_usage_at(db_path: &Path, event: &UsageEvent) {
    if !db_path.exists() {
        return;
    }
    if let Ok(db) = GraphDb::open_for_usage(db_path) {
        let _ = db.record_usage(event);
    }
}

impl ResultCount for ExploreOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.primary_symbols.len())
    }
}

impl ResultCount for AffectedOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.affected_test_files.len())
    }
}

impl ResultCount for PathOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(usize::from(self.0.is_some()))
    }
}

impl ResultCount for MissesOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.misses.len())
    }
}

impl ResultCount for FindOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.symbols.len())
    }
}

impl ResultCount for CallersOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.callers.len())
    }
}

impl ResultCount for CalleesOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.callees.len())
    }
}

impl ResultCount for FileOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.symbols.len())
    }
}

impl ResultCount for ImpactOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.affected_symbols.len())
    }
}

impl ResultCount for DeadCodeOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

impl ResultCount for ComplexityOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

impl ResultCount for HierarchyOutcome {
    fn result_count(&self) -> Option<usize> {
        Some(usize::from(self.0.is_some()))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/usage.rs"]
mod tests;
