#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::Ctx;
use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope};
use crate::store::layout::Layout;
use crate::test_support::RecordingSink;
use std::sync::Arc;

fn scope(repo: &str) -> DeliveryScope {
    DeliveryScope {
        feature: "login".into(),
        parent_feature: String::new(),
        repo: repo.into(),
        branch: "login".into(),
        head_sha: "abc".into(),
    }
}

fn pushed() -> DeliveryEvent {
    DeliveryEvent::Pushed {
        ok: true,
        detail: None,
        base: "main".into(),
    }
}

fn spooled(layout: &Layout) -> Vec<serde_json::Value> {
    std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn one_emitter_is_one_attempt_and_each_event_gets_its_own_id() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());
    let emitter = ctx.emitter(&layout);

    emitter.emit(scope("api"), pushed());
    emitter.emit(scope("web"), pushed());

    let records = sink.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].attempt_id, records[1].attempt_id);
    assert_ne!(records[0].event_id, records[1].event_id);
    let second = ctx.emitter(&layout);
    second.emit(scope("api"), pushed());
    assert_ne!(sink.records()[2].attempt_id, records[0].attempt_id);
}

#[test]
fn the_default_context_records_nothing() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    Ctx::new(root).emitter(&layout).emit(scope("api"), pushed());
    assert!(!layout.memory_spool_dir().exists());
}

#[test]
fn the_spool_sink_appends_a_sanitized_attributed_line_under_the_delivery_key() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let env = CaptureEnv {
        hall: Some(root.clone()),
        session_id: Some("sess-1".into()),
        provider: Some("omp".into()),
        feature: None,
        session_path: None,
    };
    let ctx = Ctx::new(root).with_delivery_events(Arc::new(SpoolSink::new(env)));
    let event = DeliveryEvent::Refused {
        code: "deliver.blocked".into(),
        blockers: vec!["token sk-0123456789abcdefABCDEF leaked".into()],
        fingerprint: String::new(),
    };

    ctx.emitter(&layout).emit(scope("api"), event);

    let lines = spooled(&layout);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["source_event"], "delivery.refused");
    assert_eq!(lines[0]["session_id"], "sess-1");
    assert_eq!(lines[0]["provider"], "omp");
    assert_eq!(lines[0]["feature"], "login");
    assert_eq!(lines[0]["repo"], "api");
    assert_eq!(lines[0]["native_session_id"], "");
    assert_eq!(lines[0]["importance"], 7);
    assert!(
        !lines[0]["body"]
            .as_str()
            .unwrap()
            .contains("sk-0123456789abcdefABCDEF")
    );
}

#[test]
fn outside_a_session_the_event_has_empty_session_and_provider() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let env = CaptureEnv {
        hall: Some(root.clone()),
        ..CaptureEnv::default()
    };
    let ctx = Ctx::new(root).with_delivery_events(Arc::new(SpoolSink::new(env)));

    ctx.emitter(&layout).emit(scope("api"), pushed());

    let lines = spooled(&layout);
    assert_eq!(lines[0]["session_id"], "");
    assert_eq!(lines[0]["provider"], "");
}

#[test]
fn an_unwritable_spool_never_panics_and_never_fails_the_caller() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.root().join(".ivar/spool")).unwrap();
    std::fs::write(
        layout.memory_spool_dir(),
        "a file where the directory belongs",
    )
    .unwrap();
    let sink = SpoolSink::new(CaptureEnv {
        hall: Some(root),
        ..CaptureEnv::default()
    });

    let record = DeliveryRecord {
        event_id: "e".into(),
        attempt_id: "a".into(),
        scope: scope("api"),
        event: pushed(),
    };

    assert!(sink.try_record(&layout, &record).is_err());
    sink.record(&layout, &record);
}

#[test]
fn every_selectable_provider_is_recorded_under_its_own_id_and_the_database_stays_untouched() {
    for provider in crate::domain::provider::Provider::SELECTABLE {
        let (_guard, root) = crate::test_support::seeded_hall();
        let layout = Layout::at(root.clone());
        let env = CaptureEnv {
            hall: Some(root.clone()),
            session_id: Some("sess-1".into()),
            provider: Some(provider.id().to_owned()),
            ..CaptureEnv::default()
        };
        let ctx = Ctx::new(root).with_delivery_events(Arc::new(SpoolSink::new(env)));

        ctx.emitter(&layout).emit(scope("api"), pushed());

        let lines = spooled(&layout);
        assert_eq!(lines[0]["provider"], provider.id());
        assert_eq!(lines[0]["session_id"], "sess-1");
        assert!(
            !layout.memory_db().exists(),
            "a command must not open memory.db"
        );
    }
}

#[test]
fn the_same_merge_observed_twice_has_one_event_id() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());
    let merged = || DeliveryEvent::Merged {
        number: 7,
        url: "https://x/pull/7".into(),
        merge_commit: Some("deadbeef".into()),
        merged_at: None,
        observed_by: crate::domain::delivery_event::ObservedBy::Cleanup,
    };

    ctx.emitter(&layout).emit(scope("api"), merged());
    ctx.emitter(&layout).emit(scope("api"), merged());

    let records = sink.records();
    assert_eq!(records[0].event_id, records[1].event_id);
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
}
