#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::hall::{InitInput, init};
use crate::action::sync::{SyncInput, sync};
use crate::domain::graph::{UsageEvent, UsageSource};
use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_SCHEMA_VERSION};

fn hall_for(provider: Provider) -> (tempfile::TempDir, Utf8PathBuf, Layout) {
    let (guard, root) = crate::test_support::hall_root();
    init(
        &Ctx::new(root.clone()),
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let owner = provider.config_owner();
    let available = if owner == provider {
        vec![provider]
    } else {
        vec![owner, provider]
    };
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(available, provider),
        Vec::new(),
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    (guard, root, layout)
}

fn legacy_graph_with_one_usage_row(layout: &Layout) {
    let legacy = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    legacy
        .record_usage(&UsageEvent {
            command: "graph_explore".to_owned(),
            source: UsageSource::Cli,
            duration_ms: 1,
            result_count: Some(1),
            error: false,
            session: None,
            query: Some("legacy".to_owned()),
        })
        .unwrap();
}

#[test]
fn sync_moves_the_legacy_graph_for_every_selectable_provider() {
    for provider in Provider::SELECTABLE {
        let (_guard, root, layout) = hall_for(provider);
        legacy_graph_with_one_usage_row(&layout);

        let ctx = Ctx::new(root.clone());
        let report = sync(&ctx, &SyncInput::default()).unwrap();
        assert!(report.is_clean(), "{provider}: {:?}", report.warnings);

        let graph = GraphDb::open_read_only(layout.graph_db().as_std_path())
            .unwrap_or_else(|error| panic!("{provider}: graph.db: {error}"));
        let usage_rows: i64 = graph
            .conn()
            .query_row(
                "SELECT count(*) FROM usage WHERE query = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            usage_rows, 1,
            "{provider}: the legacy usage row did not move"
        );

        let memory = MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap_or_else(|error| panic!("{provider}: memory.db: {error}"));
        assert_eq!(memory.application_id, APPLICATION_ID, "{provider}");
        assert!(!memory.polluted, "{provider}");

        let status = crate::action::memory::status::status(&ctx).unwrap().value;
        assert!(status.exists, "{provider}");
        assert_eq!(status.path, layout.memory_db(), "{provider}");
        assert_eq!(
            status.schema_version,
            Some(MEMORY_SCHEMA_VERSION),
            "{provider}"
        );
        assert_eq!(status.application_id, Some(APPLICATION_ID), "{provider}");
    }
}
