//! Delivery milestones recorded as Observations through a sink on `Ctx`.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use super::capture::{CaptureEnv, sanitize_context};
use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope};
use crate::domain::observation::{Observation, delivery_event_id, merged_event_id};
use crate::domain::sanitize::Sanitized;
use crate::domain::session::rfc3339_now;
use crate::store::layout::Layout;
use crate::store::memory::spool::{self, Appended, SpoolKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRecord {
    pub event_id: String,
    pub attempt_id: String,
    pub scope: DeliveryScope,
    pub event: DeliveryEvent,
}

pub trait DeliverySink: std::fmt::Debug + Send + Sync {
    fn record(&self, layout: &Layout, record: &DeliveryRecord);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Silent;

impl DeliverySink for Silent {
    fn record(&self, _layout: &Layout, _record: &DeliveryRecord) {}
}

#[derive(Debug)]
pub struct SpoolSink {
    env: CaptureEnv,
}

impl SpoolSink {
    #[must_use]
    pub fn new(env: CaptureEnv) -> Self {
        Self { env }
    }

    #[must_use]
    pub fn from_process() -> Self {
        Self::new(CaptureEnv::from_process())
    }

    pub(crate) fn try_record(
        &self,
        layout: &Layout,
        record: &DeliveryRecord,
    ) -> Result<(), String> {
        let observation = Observation {
            event_id: record.event_id.clone(),
            source_event: record.event.source_event().as_str().to_owned(),
            occurred_at: rfc3339_now(),
            session_id: self.env.session_id.clone().unwrap_or_default(),
            native_session_id: String::new(),
            provider: self.env.provider.clone().unwrap_or_default(),
            ivar_version: env!("CARGO_PKG_VERSION").to_owned(),
            feature: record.scope.feature.clone(),
            parent_feature: record.scope.parent_feature.clone(),
            repo: record.scope.repo.clone(),
            origin: String::new(),
            importance: record.event.importance(),
            attempt_id: record.attempt_id.clone(),
            body: record.event.body(&record.scope),
        };
        let sanitized = Sanitized::sanitize(observation, &sanitize_context(layout))
            .map_err(|_| "the event body could not be sanitized".to_owned())?;
        match spool::append(
            &layout.memory_spool_dir(),
            &SpoolKey::delivery(),
            &sanitized,
        ) {
            Ok(Appended::Written) => Ok(()),
            Ok(Appended::Dropped) => Err("the delivery spool reached its size cap".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl DeliverySink for SpoolSink {
    fn record(&self, layout: &Layout, record: &DeliveryRecord) {
        if let Err(reason) = self.try_record(layout, record) {
            let _ = writeln!(
                std::io::stderr(),
                "ivar: memory.delivery_event_not_recorded: {reason}"
            );
        }
    }
}

pub(crate) struct Emitter<'a> {
    sink: &'a dyn DeliverySink,
    layout: &'a Layout,
    attempt_id: String,
    sequence: AtomicU32,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(sink: &'a dyn DeliverySink, layout: &'a Layout) -> Self {
        Self {
            sink,
            layout,
            attempt_id: uuid::Uuid::new_v4().to_string(),
            sequence: AtomicU32::new(0),
        }
    }

    pub(crate) fn emit(&self, scope: DeliveryScope, event: DeliveryEvent) {
        let event_id = match &event {
            DeliveryEvent::Merged {
                url, merge_commit, ..
            } => merged_event_id(
                &scope.repo,
                url,
                merge_commit.as_deref().unwrap_or_default(),
            ),
            _ => delivery_event_id(
                &self.attempt_id,
                event.source_event(),
                &scope.repo,
                self.sequence.fetch_add(1, Ordering::Relaxed),
            ),
        };
        self.sink.record(
            self.layout,
            &DeliveryRecord {
                event_id,
                attempt_id: self.attempt_id.clone(),
                scope,
                event,
            },
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/delivery.rs"]
mod tests;
