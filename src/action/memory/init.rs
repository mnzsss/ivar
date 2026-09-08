//! `ivar memory init` — initialise the shared memory store structure in a hall.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall, read_manifest};
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

/// Input for the `ivar memory init` command.
#[derive(Debug, Clone, Default)]
pub struct MemoryInitInput {
    /// Optional explicit path to the hall root or memory root.
    pub path: Option<Utf8PathBuf>,
}

/// Outcome of the `ivar memory init` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryInitOutcome {
    /// The memory root directory created/initialised.
    pub root: Utf8PathBuf,
    /// Number of declared scopes created.
    pub scopes_created: usize,
}

impl WriteHuman for MemoryInitOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Initialised memory root at `{}` with {} scope(s).",
            self.root, self.scopes_created
        )
    }
}

/// Initialise memory directory structure.
pub fn init(ctx: &Ctx, input: MemoryInitInput) -> Outcome<MemoryInitOutcome> {
    let layout = if let Some(path) = input.path {
        let resolved = ctx.resolve(&path);
        match crate::store::layout::Layout::discover(&resolved)? {
            Some(layout) => layout,
            None => discover_hall(ctx)?,
        }
    } else {
        discover_hall(ctx)?
    };

    let manifest = read_manifest(&layout)?;
    let memory_root = layout.memory_root();
    fs::ensure_dir(&memory_root)?;

    // Ensure sessions directory exists under memory
    let sessions_dir = layout.memory_episodes_dir();
    fs::ensure_dir(&sessions_dir)?;

    let mut scopes_created = 0;
    if let Some(mem_config) = manifest.memory() {
        for scope in &mem_config.scopes {
            let scope_dir = layout.memory_scope_dir(&scope.id);
            if !scope_dir.exists() {
                fs::ensure_dir(&scope_dir)?;
                scopes_created += 1;
            }
        }
    }

    Ok(Report::new(MemoryInitOutcome {
        root: memory_root,
        scopes_created,
    }))
}
