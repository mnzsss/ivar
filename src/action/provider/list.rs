//! `ivar provider list` — the harnesses a hall knows about, and the default.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Outcome, Report, WriteHuman};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar provider list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every provider the hall lists.
    pub available: Vec<Provider>,
    /// The provider `ivar session start` picks when none is named.
    pub default: Provider,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let rendered = self
            .available
            .iter()
            .map(Provider::id)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            w,
            "Providers in {}: {} (default: {})",
            self.root,
            rendered,
            self.default.id()
        )
    }
}

/// List the hall's providers from `ivar.json`.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        available: manifest.providers().available().to_vec(),
        default: manifest.providers().default_provider(),
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::test_support::hall_root;

    #[test]
    fn list_reports_the_default_provider_of_a_fresh_hall() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        let report = list(&ctx).unwrap();

        assert_eq!(report.value.available, vec![Provider::ClaudeCode]);
        assert_eq!(report.value.default, Provider::ClaudeCode);
    }

    #[test]
    fn list_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = list(&ctx).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_names_available_and_default() {
        let outcome = ListOutcome {
            root: Utf8PathBuf::from("/hall"),
            available: vec![Provider::ClaudeCode, Provider::OpenCode],
            default: Provider::OpenCode,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Providers in /hall: claude-code, opencode (default: opencode)\n"
        );
    }
}
