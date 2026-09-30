#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn a_redirect_yields_its_location() {
    let location = location_from(
        302,
        Some("https://github.com/mnzsss/ivar/releases/tag/v0.13.0"),
    )
    .unwrap();
    assert_eq!(
        location,
        "https://github.com/mnzsss/ivar/releases/tag/v0.13.0"
    );
}

#[test]
fn a_non_redirect_is_a_failure_naming_the_status() {
    let failure = location_from(200, None).unwrap_err();
    assert_eq!(failure.code, "release.no_redirect");
    assert!(failure.what.contains("200"), "{}", failure.what);

    let failure = location_from(404, Some("ignored")).unwrap_err();
    assert_eq!(failure.code, "release.no_redirect");
}

#[test]
fn a_redirect_without_location_is_a_failure() {
    let failure = location_from(302, None).unwrap_err();
    assert_eq!(failure.code, "release.no_location");
}

#[test]
fn the_url_is_the_public_latest_release() {
    assert_eq!(
        LATEST_RELEASE_URL,
        "https://github.com/mnzsss/ivar/releases/latest"
    );
}
