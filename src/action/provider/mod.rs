//! `ivar provider` — which harnesses a hall knows about.
//!
//! The provider set lives in `ivar.json` (`providers.available` /
//! `providers.default`) and is materialised by `ivar sync`. `list` reports
//! it; `add` registers a provider by appending to `providers.available`,
//! leaving the default untouched. The manifest is committed and team-shared —
//! the command only ever rewrites it through the same constructors a human
//! hand-edit would have to satisfy, and `ivar sync` materialises the config
//! from it.

pub mod add;
pub mod list;
