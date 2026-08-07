//! `ivar provider` — which harnesses a hall knows about.
//!
//! The provider set lives in `ivar.json` (`providers.available` /
//! `providers.default`) and is materialised by `ivar sync`. This command
//! only *reports* it — changing the provider set is an `ivar.json` edit
//! (it is committed and team-shared, so a command that rewrites it behind
//! `git pull` would be a second writer of a file a human owns).

pub mod list;
