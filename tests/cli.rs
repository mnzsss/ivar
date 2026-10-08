//! Black-box tests that drive the compiled `ivar` binary through hall
//! lifecycle verbs outside delivery, graph and the persona scenarios.
//!
//! `tests/cli.rs` is the sole Cargo integration-test entrypoint for these
//! tests. Each scope lives in a module under `tests/cli/` and reaches the
//! shared helpers as `crate::common`.

#[path = "support/integration.rs"]
mod common;

#[path = "delivery/support.rs"]
mod delivery_support;

#[path = "cli/cli_styling.rs"]
mod cli_styling;

#[path = "cli/feature_cleanup.rs"]
mod feature_cleanup;

#[path = "cli/init.rs"]
mod init;

#[path = "cli/nested_subfeatures.rs"]
mod nested_subfeatures;

#[path = "cli/repo_create.rs"]
mod repo_create;

#[path = "cli/repo_relations.rs"]
mod repo_relations;

#[path = "cli/shipped_commands.rs"]
mod shipped_commands;

#[path = "cli/sync.rs"]
mod sync;

#[path = "cli/view_dir_lazygit.rs"]
mod view_dir_lazygit;

#[path = "cli/workspace_open.rs"]
mod workspace_open;
