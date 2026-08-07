//! Feature state on disk: `features/<name>/feature.json`.
//!
//! One file per feature, under the feature's directory, written through the
//! versioned [`Store`] with [`Policy::Local`] — this is derived local state
//! (a teammate's clone has no reason to carry another team's feature
//! worktrees), so a read that migrates persists the migrated form, silently.
//!
//! `Feature` itself carries its `version` field, the same way `Manifest`
//! does; the store stamps it on write and the type declares it, so a
//! hand-edited file with a newer version is refused rather than adopted.

use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::versioned::{Policy, Store};

/// `feature.json`'s schema version. Matches [`Feature`]'s own constant —
/// the type owns the number, this module just wires it into the store.
const CURRENT_VERSION: u32 = 1;

/// The filename every feature's promotion record lives in, under its
/// feature directory. One file, not one-per-repo: promotions are a small
/// map and rewriting one file is atomic through the canonical writer.
const FEATURE_FILE: &str = "feature.json";

impl Feature {
    /// Read `features/<name>/feature.json`. `Ok(None)` when the feature has
    /// never been written — a feature created but never promoted.
    ///
    /// A file newer than this binary understands is a hard error; see
    /// [`Store::read`].
    pub fn read(layout: &Layout, name: &FeatureName) -> Result<Option<Self>, Failure> {
        store(layout, name).read().map_err(Failure::from)
    }

    /// Write this feature to `features/<name>/feature.json`, atomically, in
    /// canonical form. Creates the feature directory if it does not exist —
    /// `feature create` calls this on a brand-new feature.
    pub fn write(&self, layout: &Layout) -> Result<(), Failure> {
        let dir = layout.feature_dir(&self.name);
        crate::infra::fs::ensure_dir(&dir)?;
        store(layout, &self.name).write(self).map_err(Failure::from)
    }
}

/// The versioned store over one feature's file.
fn store(layout: &Layout, name: &FeatureName) -> Store<Feature> {
    Store::new(
        layout.feature_dir(name).join(FEATURE_FILE),
        Vec::new(),
        CURRENT_VERSION,
        Policy::Local,
    )
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
    use crate::domain::name::{BranchName, FeatureName, RepoName};
    use crate::infra::fs;
    use crate::test_support::hall_root;

    fn layout_with_hall() -> (tempfile::TempDir, Layout) {
        let (guard, root) = hall_root();
        // A feature directory needs no ivar.json to exist, but Layout paths
        // are computed from the root alone; write a manifest so the directory
        // is a real (if empty) hall.
        let _ = root.join("ivar.json");
        (guard, Layout::at(root))
    }

    #[test]
    fn absent_feature_reads_as_ok_none() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();

        assert_eq!(Feature::read(&layout, &name).unwrap(), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let mut feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());
        feature.promote(RepoName::new("api").unwrap());

        feature.write(&layout).unwrap();
        let read_back = Feature::read(&layout, &name).unwrap().unwrap();

        assert_eq!(read_back, feature);
        assert!(read_back.is_promoted(&RepoName::new("api").unwrap()));
    }

    #[test]
    fn the_file_is_written_under_the_features_directory() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());

        feature.write(&layout).unwrap();

        assert!(fs::is_file(&layout.feature_dir(&name).join("feature.json")).unwrap());
    }

    #[test]
    fn a_file_newer_than_the_binary_is_refused() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let path = layout.feature_dir(&name).join("feature.json");
        fs::ensure_dir(path.parent().unwrap()).unwrap();
        fs::write_text(
            &path,
            r#"{"version":99,"name":"checkout","branch":"feat/checkout","promotions":{}}"#,
        )
        .unwrap();

        let error = Feature::read(&layout, &name).unwrap_err();

        assert_eq!(error.code, "store.version_too_new");
    }

    #[test]
    fn the_written_shape_is_canonical_and_version_stamped() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());

        feature.write(&layout).unwrap();

        let text = fs::read_text(&layout.feature_dir(&name).join("feature.json"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("\"version\": 1"),
            "the store must stamp the version: {text}"
        );
        assert!(text.contains("\"branch\": \"feat/checkout\""));
    }
}
