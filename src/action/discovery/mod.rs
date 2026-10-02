//! `ivar discovery …` — a unit of work's committed memory.
//!
//! Mirrors [`crate::action::plan`]'s shape: one file per verb, shared
//! loading here. Working documents live in the single feature directory
//! `.ivar/features/<name>/`.
//!
//! Unlike `plan`, these verbs never require the feature to exist: a name
//! may earn memory long before it earns execution, and often never earns
//! execution at all.

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::Ctx;
use crate::action::session::env::SessionEnv;
use crate::domain::discovery::DiscoveryDoc;
use crate::domain::name::FeatureName;
use crate::domain::session::SessionRef;
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::discovery;
use crate::store::layout::Layout;

pub mod amend;
pub mod close;
pub mod create;
pub mod list;
pub mod show;

/// Resolve the path where a discovery doc lives or should be written.
///
/// When running inside an unconverted discovery session (`feature: None`),
/// the doc resolves to `<view_dir>/discovery.md`.
/// Otherwise, it resolves to `layout.discovery_doc(name)` (`.ivar/features/<name>/discovery.md`).
pub(crate) fn resolve_doc_path(ctx: &Ctx, layout: &Layout, name: &FeatureName) -> Utf8PathBuf {
    if let Ok(Some(env)) = SessionEnv::resolve_by_cwd(&ctx.cwd)
        && env.feature.is_none()
    {
        return env.view_dir.join("discovery.md");
    }
    layout.discovery_doc(name)
}

/// Move an unconverted discovery session's doc out of its View Dir, so
/// removing the View Dir never takes the doc with it.
///
/// The doc lands where `discovery create` writes outside a session,
/// `.ivar/features/<name>/discovery.md`, named by its front matter, or by the
/// session id when that name is unusable or already holds a doc. Returns the
/// new path, or `None` when there was nothing to move.
///
/// # Errors
///
/// When the doc cannot be read or moved, or both target names are taken; the
/// caller must then keep the View Dir.
pub(crate) fn rescue_session_doc(
    layout: &Layout,
    session: &SessionRef,
) -> Result<Option<Utf8PathBuf>, Failure> {
    if session.feature.is_some() {
        return Ok(None);
    }
    let source = session.view_dir.join("discovery.md");
    let Some(text) = fs::read_text(&source)? else {
        return Ok(None);
    };
    let by_name = FeatureName::new(discovery::parse(&text).frontmatter.name).ok();
    let by_session = FeatureName::new(session.id.as_str()).ok();
    let mut free_targets = Vec::new();
    for name in [by_name, by_session].into_iter().flatten() {
        let path = layout.discovery_doc(&name);
        if !fs::exists(&path)? {
            free_targets.push(path);
        }
    }
    let Some(target) = free_targets.into_iter().next() else {
        return Err(Failure::blocked(
            "discovery.rescue_target_taken",
            format!("cannot keep the discovery doc of session `{}`", session.id),
        )
        .expected("a free `.ivar/features/<name>/discovery.md` for the doc")
        .actual(format!(
            "both candidate paths already hold a doc; `{source}` is kept"
        ))
        .fix(FixAction::safe(
            "discovery.move_by_hand",
            format!("Move `{source}` somewhere safe, then stop the session again."),
        )));
    };
    if let Some(parent) = target.parent() {
        fs::ensure_dir(parent)?;
    }
    fs::rename(&source, &target)?;
    Ok(Some(target))
}

/// Read a discovery doc at an exact path.
pub(crate) fn load_at(path: &Utf8Path, name: &FeatureName) -> Result<DiscoveryDoc, Failure> {
    if !fs::is_file(path)? {
        return Err(Failure::blocked(
            "discovery.not_found",
            format!("no discovery doc for `{name}`"),
        )
        .expected("a name with a discovery doc")
        .actual(format!("`{path}` does not exist"))
        .fix(FixAction::safe(
            "discovery.create_first",
            format!("Create it first with `ivar discovery create {name}`."),
        )));
    }
    // `is_file` above already ruled out the `None` case; treat a race as
    // an empty doc rather than panicking, and `parse` reports it unknown.
    let source = fs::read_text(path)?.unwrap_or_default();
    Ok(discovery::parse(&source))
}

/// Read a name's discovery doc from the feature directory, or fail with the
/// standard "no discovery" message.
#[cfg(test)]
pub(crate) fn load(layout: &Layout, name: &FeatureName) -> Result<DiscoveryDoc, Failure> {
    load_at(&layout.discovery_doc(name), name)
}

/// Refuse to rewrite a doc ivar could not parse.
///
/// # Errors
///
/// When the doc's front matter is unreadable (D5): rewriting it would drop
/// every key ivar failed to see.
pub(crate) fn ensure_writable(doc: &DiscoveryDoc, name: &FeatureName) -> Result<(), Failure> {
    if doc.is_writable() {
        return Ok(());
    }
    Err(Failure::blocked(
        "discovery.unreadable_frontmatter",
        format!("`{name}`'s discovery doc has front matter ivar cannot read"),
    )
    .expected("a doc with readable front matter")
    .actual("the front matter is missing, unterminated, or not a mapping")
    .fix(FixAction::safe(
        "discovery.fix_frontmatter_by_hand",
        "Repair the `---` block by hand; ivar will not rewrite a header it could not read.",
    )))
}
