use super::*;

#[test]
fn an_integrated_child_is_delivered_once_its_rebased_parent_is_delivered() {
    let (_guard, root) = delivered_parent_with_integrated_children();

    for child in ["child-a", "child-b"] {
        let preview = preview_of(&root, child);
        assert!(preview.repos[0].is_delivered, "{child}: {preview:?}");
        assert!(!preview.has_blockers(), "{child}: {preview:?}");
    }
}

#[test]
fn a_delivered_parent_does_not_see_integrated_children_as_blockers() {
    let (_guard, root) = delivered_parent_with_integrated_children();

    let preview = preview_of(&root, "checkout");

    assert!(!preview.has_blockers(), "{preview:?}");
    assert_eq!(preview.descendants.len(), 2);
    for descendant in &preview.descendants {
        assert!(descendant.repos[0].is_delivered, "{descendant:?}");
    }
}

#[test]
fn an_integrated_child_is_delivered_to_its_parent_before_any_rebase() {
    let (_guard, root) = parent_with_integrated_children();

    for child in ["child-a", "child-b"] {
        let preview = preview_of(&root, child);
        assert!(preview.repos[0].is_delivered, "{child}: {preview:?}");
    }
}

#[test]
fn an_integrated_child_is_not_delivered_while_its_rebased_parent_is_not() {
    let (_guard, root) = parent_with_integrated_children();
    rebase_parent_onto_advanced_main(&root);

    let preview = preview_of(&root, "child-b");

    assert!(!preview.repos[0].is_delivered, "{preview:?}");
    assert!(preview.has_blockers());
}

#[test]
fn an_integrated_child_that_moved_after_integration_is_not_delivered() {
    let (_guard, root) = delivered_parent_with_integrated_children();
    commit_file(&root, "child-b", "late.md");

    let preview = preview_of(&root, "child-b");

    assert!(!preview.repos[0].is_delivered, "{preview:?}");
}
