//! `ivar memory`: the hall memory database's verbs.

pub mod capture;
pub mod classify;
pub mod clean;
pub mod status;

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/provider_matrix.rs"]
mod provider_matrix;
