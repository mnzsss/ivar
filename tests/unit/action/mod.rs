#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn resolve_passes_an_absolute_path_through_unchanged() {
    let ctx = Ctx::new("/somewhere");
    assert_eq!(
        ctx.resolve(Utf8Path::new("/elsewhere")),
        Utf8PathBuf::from("/elsewhere")
    );
}

#[test]
fn resolve_joins_a_relative_path_onto_cwd() {
    let ctx = Ctx::new("/somewhere");
    assert_eq!(
        ctx.resolve(Utf8Path::new("child")),
        Utf8PathBuf::from("/somewhere/child")
    );
}

#[test]
fn resolve_treats_dot_as_cwd_itself() {
    let ctx = Ctx::new("/somewhere");
    assert_eq!(
        ctx.resolve(Utf8Path::new(".")),
        Utf8PathBuf::from("/somewhere/.")
    );
}
