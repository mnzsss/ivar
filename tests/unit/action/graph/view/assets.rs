use crate::action::graph::view::assets::{
    FONT_LICENSE, FONT_WOFF2, INDEX_HTML, VIEWER_CSS, VIEWER_JS,
};

#[test]
fn assets_are_embedded_and_non_empty() {
    assert!(!INDEX_HTML.trim().is_empty(), "INDEX_HTML must not be empty");
    assert!(!VIEWER_CSS.trim().is_empty(), "VIEWER_CSS must not be empty");
    assert!(!VIEWER_JS.trim().is_empty(), "VIEWER_JS must not be empty");
    assert!(!FONT_WOFF2.is_empty(), "FONT_WOFF2 must not be empty");
    assert!(!FONT_LICENSE.trim().is_empty(), "FONT_LICENSE must not be empty");
}

#[test]
fn font_asset_has_valid_woff2_header() {
    // WOFF2 magic header bytes: 0x774F4632 ('wOF2')
    assert!(
        FONT_WOFF2.len() >= 4,
        "FONT_WOFF2 must have at least 4 bytes"
    );
    assert_eq!(
        &FONT_WOFF2[0..4],
        b"wOF2",
        "FONT_WOFF2 must begin with the wOF2 magic header"
    );
}

#[test]
fn font_license_carries_sil_open_font_license_notice() {
    assert!(
        FONT_LICENSE.contains("SIL OPEN FONT LICENSE Version 1.1")
            || FONT_LICENSE.contains("SIL Open Font License"),
        "FONT_LICENSE must document the SIL Open Font License"
    );
    assert!(
        FONT_LICENSE.contains("Fira Code") || FONT_LICENSE.contains("Nikita Prokopov"),
        "FONT_LICENSE must attribute Fira Code"
    );
}

#[test]
fn assets_contain_no_external_urls_or_remote_dependencies() {
    for (name, content) in [
        ("INDEX_HTML", INDEX_HTML),
        ("VIEWER_CSS", VIEWER_CSS),
        ("VIEWER_JS", VIEWER_JS),
    ] {
        assert!(
            !content.contains("http://") && !content.contains("https://") && !content.contains("//"),
            "{name} must not contain external URLs or schema-relative references (offline constraint)"
        );
        assert!(
            !content.contains("unpkg.com")
                && !content.contains("jsdelivr.net")
                && !content.contains("cdnjs.cloudflare.com")
                && !content.contains("fonts.googleapis.com"),
            "{name} must not load third-party CDNs"
        );
    }
}

#[test]
fn html_contains_required_dom_structure_and_accessibility_attributes() {
    // Root and meta
    assert!(INDEX_HTML.contains("<!DOCTYPE html>"));
    assert!(INDEX_HTML.contains(r#"<html lang="en">"#));
    assert!(INDEX_HTML.contains(r#"<meta charset="utf-8">"#) || INDEX_HTML.contains(r#"<meta charset="UTF-8">"#));
    assert!(INDEX_HTML.contains(r#"<meta name="viewport""#));

    // Linked local assets
    assert!(INDEX_HTML.contains(r#"href="/viewer.css""#));
    assert!(INDEX_HTML.contains(r#"src="/viewer.js""#));

    // ARIA landmarks and controls
    assert!(INDEX_HTML.contains(r#"role="banner""#) || INDEX_HTML.contains("<header"));
    assert!(INDEX_HTML.contains(r#"role="main""#) || INDEX_HTML.contains("<main"));
    assert!(INDEX_HTML.contains(r#"role="complementary""#) || INDEX_HTML.contains("<aside"));
    assert!(INDEX_HTML.contains(r#"role="status""#) || INDEX_HTML.contains(r#"aria-live="polite""#));
    assert!(INDEX_HTML.contains(r#"id="graph-canvas""#));
    assert!(INDEX_HTML.contains(r#"aria-label="#));

    // Key interactive controls
    assert!(INDEX_HTML.contains(r#"id="search-input""#));
    assert!(INDEX_HTML.contains(r#"id="repo-filter""#));
    assert!(INDEX_HTML.contains(r#"id="kind-filter""#));
    assert!(INDEX_HTML.contains(r#"id="provenance-filter""#));
    assert!(INDEX_HTML.contains(r#"id="depth-select""#));
    assert!(INDEX_HTML.contains(r#"id="fit-btn""#));
    assert!(INDEX_HTML.contains(r#"id="reset-btn""#));
    assert!(INDEX_HTML.contains(r#"id="expand-btn""#));
}

#[test]
fn css_declares_fira_code_and_heimdall_palette_tokens() {
    // Fira Code font face with local swap and monospace fallback
    assert!(VIEWER_CSS.contains("@font-face"));
    assert!(VIEWER_CSS.contains("font-family: 'Fira Code'") || VIEWER_CSS.contains("font-family: \"Fira Code\""));
    assert!(VIEWER_CSS.contains(r#"url('/fonts/fira-code-400.woff2')"#) || VIEWER_CSS.contains(r#"url("/fonts/fira-code-400.woff2")"#));
    assert!(VIEWER_CSS.contains("font-display: swap"));

    // Heimdall color tokens
    assert!(VIEWER_CSS.contains("--graph-ground: #151515"));
    assert!(VIEWER_CSS.contains("--graph-raised: #202020"));
    assert!(VIEWER_CSS.contains("--graph-ink: #f2ebdd"));
    assert!(VIEWER_CSS.contains("--graph-ink-muted: #c8c0b2"));
    assert!(VIEWER_CSS.contains("--graph-structure: #8ba6ff"));
    assert!(VIEWER_CSS.contains("--graph-functional: #d97735"));

    // Accessibility: visible focus indicators & prefers-reduced-motion
    assert!(VIEWER_CSS.contains(":focus-visible"));
    assert!(VIEWER_CSS.contains("@media (prefers-reduced-motion: reduce)") || VIEWER_CSS.contains("prefers-reduced-motion"));
}

#[test]
fn js_uses_fixed_api_routes_and_no_continuous_animation() {
    // Exact same-origin API endpoints
    assert!(VIEWER_JS.contains("'/api/graph'") || VIEWER_JS.contains("\"/api/graph\""));
    assert!(VIEWER_JS.contains("'/api/expand'") || VIEWER_JS.contains("\"/api/expand\""));
    assert!(VIEWER_JS.contains("'/api/node'") || VIEWER_JS.contains("\"/api/node\""));
    assert!(VIEWER_JS.contains("'/api/search'") || VIEWER_JS.contains("\"/api/search\""));
    assert!(VIEWER_JS.contains("'/api/path'") || VIEWER_JS.contains("\"/api/path\""));
    assert!(VIEWER_JS.contains("'/api/impact'") || VIEWER_JS.contains("\"/api/impact\""));

    // Non-color edge semantics: dashed strokes for ambiguous provenance
    assert!(VIEWER_JS.contains("setLineDash") || VIEWER_JS.contains("[4, 4]") || VIEWER_JS.contains("[6, 4]"));

    // Simulation cooling / stopping to guarantee no continuous animation
    assert!(VIEWER_JS.contains("alpha") || VIEWER_JS.contains("stepSimulation") || VIEWER_JS.contains("settled"));
}
