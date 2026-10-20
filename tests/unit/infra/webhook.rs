#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn an_unreachable_url_is_an_error_string() {
    let error = post_json(
        "http://127.0.0.1:9/none",
        "{}",
        std::time::Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(!error.is_empty());
}
