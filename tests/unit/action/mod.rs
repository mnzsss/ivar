#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rstest::rstest;

use super::*;

#[rstest]
#[case::passes_an_absolute_path_through_unchanged("/elsewhere", "/elsewhere")]
#[case::joins_a_relative_path_onto_cwd("child", "/somewhere/child")]
#[case::treats_dot_as_cwd_itself(".", "/somewhere/.")]
fn resolve_is_relative_to_cwd(#[case] path: &str, #[case] expected: &str) {
    let ctx = Ctx::new("/somewhere");
    assert_eq!(
        ctx.resolve(Utf8Path::new(path)),
        Utf8PathBuf::from(expected)
    );
}
