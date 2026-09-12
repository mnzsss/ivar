//! Embedded static web UI assets and bundled Fira Code font for the graph viewer.

/// Embedded HTML shell (`index.html`) for the graph viewer.
pub const INDEX_HTML: &str = include_str!("assets/index.html");

/// Embedded stylesheet (`viewer.css`) with Heimdall palette and responsive layout.
pub const VIEWER_CSS: &str = include_str!("assets/viewer.css");

/// Embedded Vanilla JavaScript client concatenated from modular components.
pub const VIEWER_JS: &str = concat!(
    "(function() {\n'use strict';\n",
    include_str!("assets/viewer/state.js"),
    "\n",
    include_str!("assets/viewer/api.js"),
    "\n",
    include_str!("assets/viewer/styles.js"),
    "\n",
    include_str!("assets/viewer/transform.js"),
    "\n",
    include_str!("assets/viewer/simulation.js"),
    "\n",
    include_str!("assets/viewer/ui.js"),
    "\n",
    include_str!("assets/viewer/ui-details.js"),
    "\n",
    include_str!("assets/viewer/events.js"),
    "\n})();\n"
);

/// Embedded Cytoscape.js bundle including extensions (`cytoscape.bundle.min.js`).
pub const CYTOSCAPE_JS: &str = include_str!("assets/cytoscape.bundle.min.js");
/// Embedded standalone HTML visualizer template (`standalone.html`).
pub const STANDALONE_HTML: &str = include_str!("assets/standalone.html");

/// Bundled WOFF2 binary for Fira Code Regular (weight 400).
pub const FONT_WOFF2: &[u8] = include_bytes!("assets/fira-code-400.woff2");

/// SIL Open Font License v1.1 notice and attribution for Fira Code.
pub const FONT_LICENSE: &str = include_str!("assets/OFL.txt");

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/view/assets.rs"]
mod tests;
