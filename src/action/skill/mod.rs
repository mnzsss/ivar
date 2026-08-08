//! `ivar skill` — the hall's shared skills, under `.ivar/skills/`.
//!
//! A skill is a folder whose `SKILL.md` carries `name` and `description`
//! frontmatter; the folder's basename is the skill's id. The skills dir is
//! one of the two committed children of `.ivar/` (see
//! [`crate::store::layout::gitignore_lines`]), so a team's skills survive
//! clones.
//!
//! This slice is local-only: list and create. No valhalla/ecbert sync — that
//! is a later integration, and the `.gitignore` already treats `.ivar/skills/`
//! as committed so a future `push`/`pull` has somewhere to put things.

pub mod add;
pub mod create;
pub mod detach;
pub mod list;
pub mod remove;
pub mod status;
pub mod sync;
pub mod doctor;
pub mod update;
