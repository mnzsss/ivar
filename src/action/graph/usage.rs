use crate::action::Ctx;
use crate::action::discover_hall;
use crate::action::graph::outcome::{
    AffectedOutcome, CalleesOutcome, CallersOutcome, ComplexityOutcome, DeadCodeOutcome,
    ExploreOutcome, FileOutcome, FindOutcome, HierarchyOutcome, ImpactOutcome, PathOutcome,
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
    let db_path = layout.ivar_dir().join("memory.db");
    if let Ok(db) = GraphDb::open(db_path.as_std_path()) {
        let _ = db.record_usage(event);
    }
}

macro_rules! count_by {
    ($($outcome:ty => |$o:ident| $count:expr;)*) => {
        $(impl ResultCount for $outcome {
            fn result_count(&self) -> Option<usize> {
                let $o = self;
                Some($count)
            }
        })*
    };
}

count_by! {
    ExploreOutcome => |o| o.0.primary_symbols.len();
    AffectedOutcome => |o| o.0.affected_test_files.len();
    PathOutcome => |o| usize::from(o.0.is_some());
    FindOutcome => |o| o.symbols.len();
    CallersOutcome => |o| o.callers.len();
    CalleesOutcome => |o| o.callees.len();
    FileOutcome => |o| o.0.symbols.len();
    ImpactOutcome => |o| o.0.affected_symbols.len();
    DeadCodeOutcome => |o| o.0.len();
    ComplexityOutcome => |o| o.0.len();
    HierarchyOutcome => |o| usize::from(o.0.is_some());
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/usage.rs"]
mod tests;
