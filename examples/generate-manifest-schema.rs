//! Generate the canonical manifest schema document.
//!
//! Usage:
//! ```sh
//! cargo run --example generate-manifest-schema
//! ```
//!
//! The output is written to `schema/<version>.json` in the repository root:
//! one document per manifest version, because the schema pins `version` with
//! `const` and a single document could only ever be right for one version.
//!
//! A document for a past version is never regenerated. It describes a shape
//! that cannot change, and halls still on that version resolve their
//! `$schema` against the copy that shipped — see `docs/reference/on-disk-format.md`.

use std::fs;
use std::io::Write;

#[allow(clippy::expect_used, clippy::print_stdout)]
fn main() -> std::io::Result<()> {
    let schema = ivar::store::manifest::generate();
    let pretty = serde_json::to_string_pretty(&schema).expect("schema must serialize");
    let version = schema
        .pointer("/properties/version/const")
        .and_then(serde_json::Value::as_u64)
        .expect("generated schema must pin a version");

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/schema");
    fs::create_dir_all(dir)?;
    let path = format!("{dir}/{version}.json");

    let mut file = fs::File::create(&path)?;
    file.write_all(pretty.as_bytes())?;
    file.write_all(b"\n")?;
    println!("Wrote {path}");
    Ok(())
}
