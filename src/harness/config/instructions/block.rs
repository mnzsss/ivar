use std::fmt::Write as _;

use super::{MANAGED_END, MANAGED_START};
use crate::domain::name::{HallName, RepoName};
use crate::domain::session::SCRATCH_DIR;

/// Build the managed block for a hall named `hall` containing `repos`.
///
/// Pure: no I/O, no clock, no environment. Two calls with the same arguments
/// produce the same bytes, which is what lets [`super::materialise`] decide
/// "unchanged" by comparison instead of by bookkeeping.
///
/// Repos are listed in the order given — manifest order, which is the order the
/// user wrote them in and therefore the order they expect to read them back.
#[must_use]
pub fn build_block(hall: &HallName, repos: &[RepoName]) -> String {
    let mut block = String::new();

    block.push_str(MANAGED_START);
    block.push('\n');
    let _ = write!(block, "# {hall}\n\n");
    block.push_str(
        "This directory is an `ivar` hall. Each repository below is a real git\n\
         worktree mounted under `.ivar/repos/`, so a change here is a change in\n\
         that repository — there is no copy and no sync step to remember.\n\n\
         After pulling this hall, run `ivar sync` to bring the local checkout\n\
         back in line with `ivar.json`.\n\n",
    );

    block.push_str("## Repositories\n\n");
    if repos.is_empty() {
        block.push_str(
            "None yet. Add one to the `repos` list in `ivar.json`, then run `ivar sync`.\n",
        );
    } else {
        for repo in repos {
            block.push_str("- `");
            block.push_str(repo.as_str());
            block.push_str("`\n");
        }
    }

    block.push_str("## For AI Agents\n\n");
    block.push_str(
        "This directory is an `ivar` orchestration hall. You are fully authorized to\n\
         run `ivar` commands (e.g., `ivar feature create`, `ivar session start`, `ivar sync`)\n\
         on behalf of the user to manage features, repos, and sessions.\n\n\
         If you are unsure of the exact CLI syntax or how `ivar` works, DO NOT GUESS.\n\
         First, fetch and read the documentation from:\n\
         https://ivar.run/llms.txt\n",
    );
    let _ = write!(
        block,
        "\nFor edit/write, use absolute paths inside the current session's worktree\n\
         or view dir. Store temporary files in the session's `{SCRATCH_DIR}/` rather\n\
         than a harness scratchpad.\n\n\
         Write temporary and working files in the current session's scratch\n\
         directory — `{SCRATCH_DIR}/` inside the session's own view dir\n\
         (`.ivar/sessions/<session-id>/{SCRATCH_DIR}/` for a discovery session,\n\
         `.ivar/features/<feature>/sessions/<session-id>/{SCRATCH_DIR}/` once it is\n\
         bound to a feature). The rest of the hall outside `.ivar/` — docs,\n\
         `HALL.md`, harness config — is writable too, so hall files need no\n\
         feature. The hall's git hooks and config, the provider hook\n\
         configuration that wires `ivar guard`, the MCP config, and `.env`\n\
         stay protected. Repos stay read-only until promoted, and a denied\n\
         write names the writable set for you.\n\n\
         Its contents are temporary: `ivar session stop` removes the view dir and\n\
         everything in it. Nothing you need to keep belongs there.\n"
    );

    block.push('\n');
    block.push_str(MANAGED_END);
    block
}
