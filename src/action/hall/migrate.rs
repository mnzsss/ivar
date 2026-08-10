//! `ivar migrate` — advance `ivar.json`'s schema version, asking first.

// ---------------------------------------------------------------------------
// `ivar migrate` — advance ivar.json's schema version, asking first.
// ---------------------------------------------------------------------------

/// The outcome for every plan that leaves the file exactly as it was — three of
/// the four, plus a declined prompt.
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::store::manifest::{Manifest, MigrationPlan};

use super::Ctx;
use super::{ask, discover_hall};

fn untouched(manifest: Utf8PathBuf, plan: MigrationPlan) -> MigrateOutcome {
    MigrateOutcome {
        manifest,
        plan,
        migrated: false,
    }
}

/// What `ivar migrate` found, and whether it acted.
#[derive(Debug, Clone, Serialize)]
pub struct MigrateOutcome {
    /// The file this is about. Always the hall's `ivar.json`.
    pub manifest: Utf8PathBuf,
    /// What migrating would do, decided before anything was written.
    pub plan: MigrationPlan,
    /// Whether the file was actually rewritten. `false` whenever the plan had
    /// nothing to do, and also when a human declined or nobody was asked.
    pub migrated: bool,
}

impl WriteHuman for MigrateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self.plan {
            MigrationPlan::Current { version } => {
                writeln!(
                    w,
                    "{} is at version {version}. Nothing to do.",
                    self.manifest
                )
            }
            MigrationPlan::Available { from, to } if self.migrated => {
                writeln!(w, "Migrated {} from version {from} to {to}.", self.manifest)?;
                writeln!(
                    w,
                    "Commit it: your teammates need this change before their ivar can write the file."
                )
            }
            MigrationPlan::Available { from, to } => {
                writeln!(
                    w,
                    "{} would migrate from version {from} to {to}.",
                    self.manifest
                )?;
                writeln!(w, "Nothing was written.")
            }
            MigrationPlan::Unreachable { from, to } => writeln!(
                w,
                "{} reports version {from}, and this build has no migration to reach version {to}.",
                self.manifest
            ),
            MigrationPlan::TooNew { found, highest } => writeln!(
                w,
                "{} is at version {found}; this build understands up to {highest}.",
                self.manifest
            ),
        }
    }
}

/// Advance `ivar.json`'s on-disk schema version, after showing what would
/// change and asking.
///
/// This verb exists because [`crate::store::versioned::Policy::Committed`]
/// refuses to advance a committed file on its own, and its refusal names this
/// command as the way forward. `ivar.json` is in git: rewriting it during
/// somebody's unrelated `ivar sync` would put a schema bump in their next
/// commit and break a teammate still on the older binary. So it is a team
/// event with a human behind it.
///
/// Interactive by the same rule as [`cleanup`], and for the same reason: on a
/// non-tty run it prints the plan and writes nothing, so a script or an agent
/// can never advance a committed file unattended. There is deliberately no
/// `--yes`.
///
/// Local state (`.ivar/state.json`, lockfiles) is not this verb's business —
/// it migrates itself silently on read, because nobody reviews it. See
/// `docs/reference/on-disk-format.md`.
pub fn migrate(ctx: &Ctx) -> Outcome<MigrateOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest_path = layout.manifest().to_path_buf();

    // `plan` deliberately, not `read_manifest`: a too-new or unreachable file
    // is exactly what this verb has to be able to describe, and reading it
    // would refuse before there was anything to say.
    let plan = Manifest::plan(&layout)?.ok_or_else(|| {
        Failure::blocked(
            "hall.manifest_vanished",
            format!("`{manifest_path}` disappeared while reading it"),
        )
        .expected("the ivar.json that was there a moment ago")
        .actual("it is gone")
        .fix(FixAction::safe("hall.retry", "Run the command again.").command("ivar migrate"))
    })?;

    // One destructure decides the whole verb: `Available` is the only plan with
    // work in it, and each of the other three reports differently.
    //
    // The two that describe an unusable hall carry a warning. Describing it is
    // the job; exiting 0 while doing it is not — nothing was refused and
    // nothing broke, so this is the warning channel (exit 1), not a `Failure`.
    let (from, to) = match plan {
        MigrationPlan::Available { from, to } => (from, to),
        MigrationPlan::Current { .. } => {
            return Ok(Report::new(untouched(manifest_path, plan)));
        }
        MigrationPlan::TooNew { found, highest } => {
            let warning = Warning::new(
                "hall.manifest_too_new",
                manifest_path.to_string(),
                format!(
                    "schema version {found}, but this build understands up to {highest} — upgrade ivar; this command cannot help"
                ),
            );
            return Ok(Report::with_warnings(
                untouched(manifest_path, plan),
                vec![warning],
            ));
        }
        MigrationPlan::Unreachable { from, to } => {
            let warning = Warning::new(
                "hall.manifest_unreachable",
                manifest_path.to_string(),
                format!(
                    "schema version {from} with no migration to version {to} — check this is the file you meant; a file at a version this format never had is not one ivar wrote"
                ),
            );
            return Ok(Report::with_warnings(
                untouched(manifest_path, plan),
                vec![warning],
            ));
        }
    };

    let question = format!("Migrate `{manifest_path}` from version {from} to {to}?");
    if !ask(
        &question,
        "migrate.write_prompt",
        "migrate.read_answer",
        Some(
            "This rewrites a committed file. Commit the result — a teammate on an older ivar will refuse it until they upgrade.",
        ),
    )? {
        return Ok(Report::new(untouched(manifest_path, plan)));
    }

    Manifest::migrate(&layout)?;

    Ok(Report::new(MigrateOutcome {
        manifest: manifest_path,
        plan,
        migrated: true,
    }))
}
