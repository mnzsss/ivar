//! `ivar provider add <name>` — register a provider in `ivar.json`.
//!
//! Appends `name` to `providers.available`, leaving `providers.default`
//! untouched — adding a harness never silently changes which one sessions
//! start in. The manifest is the committed record, and the new provider's
//! shipped workflow commands are materialised immediately; `ivar sync` is what
//! materialises the rest of its config (managed block, MCP) from the manifest.
//!
//! Two refusals, both before anything is written: an unknown provider id
//! (the set is closed — `claude-code` and `opencode`), and a provider that is
//! already registered. Neither touches `ivar.json`.
//!
//! A command-write failure is a **warning**, not a failure: the manifest keeps
//! the provider, and the warning names `ivar sync` as the repair.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::store::manifest::{Manifest, Providers};

use super::super::sync;
use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar provider add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    /// The provider's id — `claude-code` or `opencode`.
    pub name: String,
}

/// What `ivar provider add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The provider that was registered.
    pub provider: Provider,
    /// Every provider the hall now lists, in id order.
    pub available: Vec<Provider>,
    /// The hall's default provider — unchanged by this command.
    pub default: Provider,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let rendered = self
            .available
            .iter()
            .map(Provider::id)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            w,
            "Registered provider `{}` in {} — available: {} (default: {}).",
            self.provider, self.root, rendered, self.default
        )
    }
}

/// Register `input.name` in the hall's `providers.available`.
///
/// Blocked when the provider id is unknown or already registered — in both
/// cases `ivar.json` is left untouched.
pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    // Unknown providers are refused — the set is closed, and a typo must not
    // become a silent no-op or a new, unbuildable harness.
    let provider: Provider = input.name.parse()?;

    // Duplicates are refused, never silently collapsed.
    if manifest.providers().available().contains(&provider) {
        return Err(Failure::blocked(
            "provider.already_available",
            format!("`{provider}` is already registered in ivar.json"),
        )
        .expected("a provider not yet in `providers.available`")
        .actual(format!("`{provider}` is already listed"))
        .fix(FixAction::safe(
            "provider.use_existing",
            format!("Nothing to do — `{provider}` is already available."),
        )));
    }

    let default = manifest.providers().default_provider();
    let mut available = manifest.providers().available().to_vec();
    available.push(provider);
    available.sort();

    // Rebuild the manifest through its constructors so the MCP servers are
    // preserved — `Manifest::new` alone would drop them.
    let updated = Manifest::new(
        manifest.name().clone(),
        Providers::new(available.clone(), default),
        manifest.repos().to_vec(),
        manifest.skills().cloned(),
    )?
    .with_mcp_servers(manifest.mcp_servers().to_vec())?;
    Manifest::write(&layout, &updated)?;

    // The provider is registered and its commands are bootstrapped in the same
    // run — no follow-up sync needed for them. A write failure is a warning
    // (the manifest keeps the provider; sync is the repair), never a rollback.
    let mut report = Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        provider,
        available,
        default,
    });
    if let Err(warning) = sync::materialise_commands(&layout, provider) {
        report.warn(warning);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::error::Status;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
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
        (guard, root)
    }

    fn add_input(name: &str) -> AddInput {
        AddInput {
            name: name.to_owned(),
        }
    }

    /// The providers recorded in ivar.json, read back off disk.
    fn persisted_available(root: &Utf8PathBuf) -> Vec<Provider> {
        let layout = Layout::at(root.clone());
        Manifest::read(&layout)
            .unwrap()
            .unwrap()
            .providers()
            .available()
            .to_vec()
    }

    #[test]
    fn add_registers_a_provider_and_keeps_the_default() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = add(&ctx, add_input("opencode")).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.provider, Provider::OpenCode);
        assert_eq!(
            report.value.available,
            vec![Provider::ClaudeCode, Provider::OpenCode]
        );
        assert_eq!(report.value.default, Provider::ClaudeCode);
        assert_eq!(
            persisted_available(&root),
            vec![Provider::ClaudeCode, Provider::OpenCode],
            "ivar.json must record the new provider"
        );
    }

    #[test]
    fn provider_add_materialises_the_new_providers_commands() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = add(&ctx, add_input("opencode")).unwrap();

        assert!(report.is_clean());
        for command in crate::harness::commands::catalog() {
            assert!(
                std::path::Path::new(&root)
                    .join(".opencode/commands")
                    .join(command.file_name())
                    .is_file(),
                "{} should be materialised immediately, without a follow-up sync",
                command.file_name()
            );
        }
    }

    #[test]
    fn provider_add_returns_warning_when_commands_cannot_be_written() {
        let (_guard, root) = seeded_hall();
        // Occupy OpenCode's command-directory parent with a regular file, so
        // `ensure_dir` cannot create `.opencode/commands` under it.
        fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
        let ctx = Ctx::new(root.clone());

        let report = add(&ctx, add_input("opencode")).unwrap();

        assert!(!report.is_clean(), "a failed command write must not be clean");
        assert_eq!(report.warnings[0].code, "provider.commands_not_materialised");
        assert_eq!(report.warnings[0].subject, "opencode");
        assert!(report.warnings[0].what.contains("ivar sync"));
        assert_eq!(
            persisted_available(&root),
            vec![Provider::ClaudeCode, Provider::OpenCode],
            "the provider must stay registered even when its commands could not be written"
        );
    }

    #[test]
    fn add_refuses_an_unknown_provider() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let failure = add(&ctx, add_input("bogus")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "provider.unknown_id");
        assert!(
            failure.what.contains("claude-code") && failure.what.contains("opencode"),
            "the refusal must name the valid ids: {}",
            failure.what
        );
        assert_eq!(
            persisted_available(&root),
            vec![Provider::ClaudeCode],
            "an unknown provider must not touch ivar.json"
        );
    }

    #[test]
    fn add_does_not_duplicate_an_existing_provider() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, add_input("opencode")).unwrap();

        let failure = add(&ctx, add_input("opencode")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "provider.already_available");
        assert_eq!(
            persisted_available(&root),
            vec![Provider::ClaudeCode, Provider::OpenCode],
            "a duplicate add must not change ivar.json"
        );
    }

    #[test]
    fn add_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = add(&ctx, add_input("opencode")).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_names_the_registered_provider() {
        let outcome = AddOutcome {
            root: Utf8PathBuf::from("/hall"),
            provider: Provider::OpenCode,
            available: vec![Provider::ClaudeCode, Provider::OpenCode],
            default: Provider::ClaudeCode,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Registered provider `opencode` in /hall — available: claude-code, opencode \
             (default: claude-code).\n"
        );
    }
}
