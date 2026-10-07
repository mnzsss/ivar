//! Unit tests for `crate::action::session::instructions`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;
use serde_json::json;

fn touched(input: serde_json::Value, cwd: &str) -> Vec<String> {
    touched_paths(&input, Utf8Path::new(cwd))
        .into_iter()
        .map(String::from)
        .collect()
}

#[test]
fn file_fields_resolve_lexically_against_the_cwd_which_always_counts() {
    assert_eq!(
        touched(json!({ "file_path": "api/src/lib.rs" }), "/v"),
        ["/v/api/src/lib.rs", "/v"]
    );
    assert_eq!(
        touched(json!({ "filePath": "/v/web/a.ts" }), "/v"),
        ["/v/web/a.ts", "/v"]
    );
    assert_eq!(
        touched(json!({ "path": "api/../web/./x" }), "/v"),
        ["/v/web/x", "/v"]
    );
    assert_eq!(touched(json!({ "pattern": "**/*.rs" }), "/v"), ["/v"]);
}

#[test]
fn shell_command_words_are_touched_paths() {
    let cases: [(&str, &[&str]); 9] = [
        // cat rel
        ("cat api/README.md", &["/v/api/README.md", "/v"]),
        // cd chain: the cd target counts and later words resolve against it
        (
            "cd api/src && ls ../docs",
            &["/v/api/src", "/v/api/docs", "/v"],
        ),
        // abs + pipe: words after `|` count too
        (
            "cat /v/api/a.md | tee web/out.txt",
            &["/v/api/a.md", "/v/web/out.txt", "/v"],
        ),
        // git -C: every positional word counts; a non-path word like `status` names a
        // missing path that instruction_chain later reduces to its deepest existing dir
        ("git -C api status", &["/v/api", "/v/status", "/v"]),
        // --opt=path
        (
            "rustfmt --config-path=api/rustfmt.toml",
            &["/v/api/rustfmt.toml", "/v"],
        ),
        // nonexistent deeper: returned lexically, never checked here
        ("mkdir -p api/new/deep", &["/v/api/new/deep", "/v"]),
        // outside the view: returned as-is, filtered by instruction_chain
        ("cat /etc/hosts", &["/etc/hosts", "/v"]),
        // parentheses separate words
        ("(cd web && ls src)", &["/v/web", "/v/web/src", "/v"]),
        // `$`, backtick and `~` words are skipped
        (
            "cat $HOME/x \"$(pwd)\"/y `pwd`/z ~/w api/ok",
            &["/v/api/ok", "/v"],
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(
            touched(json!({ "command": command }), "/v"),
            expected,
            "{command}"
        );
    }
}

#[test]
fn workdir_and_cwd_fields_move_the_command_base() {
    assert_eq!(
        touched(
            json!({ "command": "cat lib.rs", "workdir": "api/src" }),
            "/v"
        ),
        ["/v/api/src/lib.rs", "/v/api/src"]
    );
    assert_eq!(
        touched(json!({ "command": "ls", "cwd": "/v/web" }), "/v"),
        ["/v/web"]
    );
}

#[test]
fn heredoc_bodies_are_not_read_as_commands() {
    let paths = touched(
        json!({ "command": "cat > api/notes.md <<'EOF'\ncat web/secret.md\nEOF" }),
        "/v",
    );
    assert!(paths.contains(&"/v/api/notes.md".to_owned()), "{paths:?}");
    assert!(!paths.iter().any(|p| p.starts_with("/v/web")), "{paths:?}");
}

#[test]
fn glob_words_expand_through_the_filesystem_up_to_the_cap() {
    let (_dir, view) = utf8_temp_dir();
    let src = view.join("api/src");
    fs::ensure_dir(&src).unwrap();
    for name in ["a.rs", "b.rs", ".hidden.rs", "c.txt"] {
        fs::write_text(&src.join(name), "").unwrap();
    }
    let cwd = view.as_str();
    let at = |rel: &str| view.join(rel).into_string();

    assert_eq!(
        touched(json!({ "command": "cat api/src/*.rs" }), cwd),
        [at("api/src/a.rs"), at("api/src/b.rs"), view.to_string()]
    );
    assert_eq!(
        touched(json!({ "command": "cat api/src/[ab].rs api/*/?.txt" }), cwd),
        [
            at("api/src/a.rs"),
            at("api/src/b.rs"),
            at("api/src/c.txt"),
            view.to_string()
        ]
    );
    // no match: the directory before the first wildcard counts
    assert_eq!(
        touched(json!({ "command": "cat api/src/*.md" }), cwd),
        [at("api/src"), view.to_string()]
    );

    let generated = view.join("api/gen");
    fs::ensure_dir(&generated).unwrap();
    for n in 0..GLOB_CAP + 10 {
        fs::write_text(&generated.join(format!("f{n}.rs")), "").unwrap();
    }
    let hits = touched(json!({ "command": "ls api/gen/f*" }), cwd);
    assert_eq!(
        hits.iter()
            .filter(|p| p.starts_with(&at("api/gen/")))
            .count(),
        GLOB_CAP
    );
}
