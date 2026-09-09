//! Embedded static web UI assets and bundled Fira Code font for the graph viewer.

/// Embedded HTML shell (`index.html`) for the graph viewer.
pub const INDEX_HTML: &str = include_str!("assets/index.html");

/// Embedded stylesheet (`viewer.css`) with Heimdall palette and responsive layout.
pub const VIEWER_CSS: &str = include_str!("assets/viewer.css");

/// Embedded Vanilla JavaScript client (`viewer.js`) for Canvas 2D interaction and rendering.
pub const VIEWER_JS: &str = include_str!("assets/viewer.js");

/// Embedded Cytoscape.js bundle including extensions (`cytoscape.bundle.min.js`).
pub const CYTOSCAPE_JS: &str = include_str!("assets/cytoscape.bundle.min.js");

/// Bundled WOFF2 binary for Fira Code Regular (weight 400).
pub const FONT_WOFF2: &[u8] = include_bytes!("assets/fira-code-400.woff2");

/// SIL Open Font License v1.1 notice and attribution for Fira Code.
pub const FONT_LICENSE: &str = include_str!("assets/OFL.txt");

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/view/assets.rs"]
mod tests;
