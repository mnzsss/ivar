#![allow(clippy::unwrap_used)]

use crate::common::{hall_root, ivar};

#[test]
fn sync_renames_the_graph_entry_once_and_a_second_sync_changes_nothing() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest.as_object_mut().unwrap().insert(
        "mcp".to_owned(),
        serde_json::json!([
            {"name": "graph", "type": "local", "command": "ivar", "args": ["graph", "mcp"]}
        ]),
    );
    std::fs::write(&path, manifest.to_string()).unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();
    let first = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(
        first.contains(r#""name": "ivar""#) || first.contains(r#""name":"ivar""#),
        "{first}"
    );
    let config = std::fs::read_to_string(root.join(".mcp.json")).unwrap();
    assert!(
        config.contains("hall-ivar") && !config.contains("hall-graph"),
        "{config}"
    );

    ivar().current_dir(&root).arg("sync").assert().success();
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        first
    );
}
