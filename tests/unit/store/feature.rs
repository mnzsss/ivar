#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::domain::name::{BranchName, RepoName};
use crate::test_support::utf8_temp_dir;

#[test]
fn concurrent_updates_to_one_feature_keep_every_write() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let name = FeatureName::new("parent").unwrap();
    Feature::new(name.clone(), BranchName::new("parent").unwrap())
        .write(&layout)
        .unwrap();

    std::thread::scope(|scope| {
        for i in 0..8 {
            let (layout, name) = (&layout, &name);
            scope.spawn(move || {
                Feature::update(layout, name, |feature| {
                    feature.promote(RepoName::new(format!("repo{i}")).unwrap());
                    Ok(())
                })
                .unwrap();
            });
        }
    });

    let feature = Feature::read_or_not_found(&layout, &name).unwrap();
    assert_eq!(feature.promotions.len(), 8);
}
