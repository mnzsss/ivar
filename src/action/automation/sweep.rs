//! The part of the hourly memory sweep that maintains the automation
//! decisions. Accepted decisions never expire.

use rusqlite::{Transaction, TransactionBehavior};

use crate::domain::automation::{needs_groups, plan_sweep, utc_day};
use crate::domain::repetition::WINDOW_DAYS;
use crate::store::memory::automation as store;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::fingerprints::grouped_since;
use crate::store::memory::retention::cutoff_day_before;

/// What the sweep did to the decisions.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DecisionsSwept {
    /// Declined decisions no fingerprint of the window matches any more, and
    /// deferrals past their `until`: deleted.
    pub expired: usize,
    /// Decisions that took texts from the groups that currently match them.
    pub extended: usize,
}

/// Extends each declined and deferred decision with the texts of the groups
/// that currently match it, deletes a declined decision that no fingerprint of
/// the 90-day window matches, and deletes a deferral past its `until`. Run it
/// after the fingerprints of the window have been pruned.
///
/// # Errors
///
/// When `SQLite` fails; nothing is changed then.
pub fn sweep_decisions(db: &mut MemoryDb, now_epoch_secs: u64) -> rusqlite::Result<DecisionsSwept> {
    let tx = Transaction::new(db.conn_mut(), TransactionBehavior::Immediate)?;
    let decisions = store::list(&tx)?;
    let today = utc_day(now_epoch_secs);
    let groups: Vec<_> = if needs_groups(&decisions, &today) {
        grouped_since(&tx, &cutoff_day_before(now_epoch_secs, WINDOW_DAYS))?
            .1
            .iter()
            .map(|group| group.texts().into_iter().map(str::to_owned).collect())
            .collect()
    } else {
        Vec::new()
    };
    let plan = plan_sweep(&decisions, &groups, &today);
    for (id, texts) in &plan.extend {
        store::add_texts(&tx, *id, texts)?;
    }
    for id in &plan.delete {
        store::delete(&tx, *id)?;
    }
    tx.commit()?;
    Ok(DecisionsSwept {
        expired: plan.delete.len(),
        extended: plan.extend.len(),
    })
}
