//! Checks that read the repository instead of running the binary: the
//! architecture rules, the generated command reference and documentation
//! links, the skill-sync golden vectors, and the shipped prose.
//!
//! `tests/static_checks.rs` is the sole Cargo integration-test entrypoint for
//! these checks. Each one lives in a module under `tests/static_checks/`.

#[path = "static_checks/architecture.rs"]
mod architecture;

#[path = "static_checks/docs_reference.rs"]
mod docs_reference;

#[path = "static_checks/shipped_text.rs"]
mod shipped_text;

#[path = "static_checks/skill.rs"]
mod skill;
