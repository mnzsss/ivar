#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::graph::index_cmd;
use crate::action::graph::input::IndexInput;
use crate::action::hall::{InitInput, init};
use crate::store::layout::Layout;

#[test]
fn index_warns_and_keeps_layers_when_the_features_directory_is_unreadable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let features_dir = Layout::at(root.clone()).features_dir();
    let _ = std::fs::remove_dir_all(&features_dir);
    std::fs::write(&features_dir, b"not a directory").unwrap();

    let report = index_cmd(
        &ctx,
        IndexInput {
            repo: None,
            full: false,
            register_mcp: false,
            paths: vec![],
        },
    )
    .unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "graph.reconcile_skipped"),
        "warnings: {:?}",
        report.warnings
    );
    assert!(report.value.removed.layers.is_empty());
}
