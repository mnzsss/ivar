//! `ivar provider` — which harnesses a hall knows about.
//!
//! The provider set lives in `ivar.json` (`providers.available` /
//! `providers.default`) and is materialised by `ivar sync`. `list` reports
//! it; `add` registers a provider by appending to `providers.available`,
//! leaving the default untouched; `default` switches the default to a
//! registered one; `remove` drops one and tears down its config. The manifest is committed and team-shared —
//! the command only ever rewrites it through the same constructors a human
//! hand-edit would have to satisfy, and `ivar sync` materialises the config
//! from it.

pub mod add;
pub mod default;
pub mod list;
pub mod remove;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

pub(super) fn not_available(provider: Provider) -> Failure {
    Failure::blocked(
        "provider.not_available",
        format!("`{provider}` is not registered in ivar.json"),
    )
    .expected("a provider listed in `providers.available`")
    .actual(format!("`{provider}` is not listed"))
    .fix(
        FixAction::safe("provider.list", "See the registered providers.")
            .command("ivar provider list"),
    )
}
