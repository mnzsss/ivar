//! Generate the canonical `ivar.schema.json` artifact.
//!
//! Usage:
//! ```sh
//! cargo run --example generate-manifest-schema
//! ```
//!
//! The output is written to `ivar.schema.json` in the repository root.

use std::fs;
use std::io::Write;

#[allow(clippy::expect_used, clippy::print_stdout)]
fn main() -> std::io::Result<()> {
    let schema = ivar::store::manifest::generate();
    let pretty = serde_json::to_string_pretty(&schema).expect("schema must serialize");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/ivar.schema.json");
    let mut file = fs::File::create(path)?;
    file.write_all(pretty.as_bytes())?;
    file.write_all(b"\n")?;
    println!("Wrote {path}");
    Ok(())
}
