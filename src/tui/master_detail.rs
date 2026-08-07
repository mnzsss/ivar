//! Bridge between the hall's real state and the widget's [`Snapshot`].
//!
//! The widget is a pure projection; this module is where "what the hall
//! looks like" becomes "what the TUI shows". It holds **no I/O and no
//! `store`** — `tui` may not import `store` (ARCHITECTURE.md's layering
//! table), so the host loop (`action::session`) reads the hall and hands
//! ready-made [`Row`]s in. This module only arranges them.

use super::key_router::Mode;
use super::widget::{Row, Snapshot};

/// Build the master-detail [`Snapshot`] from state the host loop prepared.
///
/// `rows` are ready-made (the host read the hall); `detail` is the selected
/// feature's promotion detail; `agent_text` is the PTY scrollback the driver
/// captured.
#[must_use]
pub fn snapshot(
    root: &str,
    rows: Vec<Row>,
    selected: usize,
    detail: &str,
    agent_text: &str,
    mode: Mode,
) -> Snapshot {
    Snapshot {
        root: root.to_owned(),
        rows,
        selected,
        detail: detail.to_owned(),
        agent_scrollback: agent_text.to_owned(),
        mode,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                label: "checkout".to_owned(),
                status: "0/1".to_owned(),
            },
            Row {
                label: "fresh".to_owned(),
                status: "empty".to_owned(),
            },
        ]
    }

    #[test]
    fn snapshot_carries_rows_statuses_and_mode() {
        let snapshot = snapshot("/hall", rows(), 1, "detail", "agent", Mode::Navigate);

        assert_eq!(snapshot.root, "/hall");
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.rows[0].status, "0/1");
        assert_eq!(snapshot.selected, 1);
        assert_eq!(snapshot.detail, "detail");
        assert_eq!(snapshot.agent_scrollback, "agent");
        assert_eq!(snapshot.mode, Mode::Navigate);
    }
}
