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

fn write(path: &Utf8Path, text: &str) {
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(path, text).unwrap();
}

/// A view dir `<tmp>/view` whose repo `api` is a symlink to
/// `<tmp>/worktrees/api-main`, which holds an empty `src/deep/`.
fn view_with_api() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (dir, root) = utf8_temp_dir();
    let worktree = root.join("worktrees/api-main");
    fs::ensure_dir(&worktree.join("src/deep")).unwrap();
    let view = root.join("view");
    fs::ensure_dir(&view).unwrap();
    fs::create_symlink(&worktree, &view.join("api")).unwrap();
    (dir, view, worktree)
}

fn label(file: &Utf8Path) -> String {
    format!(
        "Repository instructions from {file} (they apply to work under {}):\n",
        file.parent().unwrap()
    )
}

#[test]
fn chain_runs_from_the_repo_root_down_to_the_deepest_existing_directory() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    write(&worktree.join("src/CLAUDE.md"), "src");
    let expected = [view.join("api/CLAUDE.md"), view.join("api/src/CLAUDE.md")];
    for touched in [
        "api/src/deep/lib.rs",
        "api/src/deep",
        "api/src/deep/missing/more/x.rs",
        "api/src/CLAUDE.md",
    ] {
        assert_eq!(
            instruction_chain(&view, &view.join(touched), Provider::ClaudeCode),
            expected,
            "{touched}"
        );
    }
    assert_eq!(
        instruction_chain(&view, &view.join("api"), Provider::ClaudeCode),
        [view.join("api/CLAUDE.md")]
    );
}

#[test]
fn each_directory_offers_the_provider_native_file_else_the_other() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "claude root");
    write(&worktree.join("AGENTS.md"), "agents root");
    write(&worktree.join("src/AGENTS.md"), "agents src");
    write(&worktree.join("src/deep/CLAUDE.md"), "claude deep");
    let touched = view.join("api/src/deep/lib.rs");
    assert_eq!(
        instruction_chain(&view, &touched, Provider::ClaudeCode),
        [
            view.join("api/CLAUDE.md"),
            view.join("api/src/AGENTS.md"),
            view.join("api/src/deep/CLAUDE.md")
        ]
    );
    for provider in [Provider::Omp, Provider::OpenCode] {
        assert_eq!(
            instruction_chain(&view, &touched, provider),
            [
                view.join("api/AGENTS.md"),
                view.join("api/src/AGENTS.md"),
                view.join("api/src/deep/CLAUDE.md")
            ],
            "{provider:?}"
        );
    }
}

#[test]
fn paths_outside_a_linked_repo_have_no_chain_and_deliver_nothing() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    write(&view.join("CLAUDE.md"), "hall root file");
    write(&view.join(".claude/CLAUDE.md"), "config dir");
    let outside = [
        Utf8PathBuf::from("/etc/hosts"),
        view.clone(),
        view.join("CLAUDE.md"),
        view.join("missing/x.rs"),
        view.join(".claude/x"),
        // the worktree itself, not reached through the view symlink
        worktree.join("src/lib.rs"),
    ];
    for path in &outside {
        assert!(
            instruction_chain(&view, path, Provider::ClaudeCode).is_empty(),
            "{path}"
        );
    }
    assert_eq!(
        deliver(&view, Provider::ClaudeCode, "main", &outside).unwrap(),
        None
    );
    assert!(!fs::exists(&state_dir(&view, Provider::ClaudeCode)).unwrap());
}

#[test]
fn deliver_returns_unseen_files_whole_once_per_agent() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root rules\n");
    write(&worktree.join("src/CLAUDE.md"), "src rules\n");
    let touched = [view.join("api/src/lib.rs"), view.join("api/README.md")];
    let expected = format!(
        "{}root rules\n\n\n{}src rules\n",
        label(&view.join("api/CLAUDE.md")),
        label(&view.join("api/src/CLAUDE.md"))
    );
    assert_eq!(
        deliver(&view, Provider::ClaudeCode, "main", &touched).unwrap(),
        Some(expected)
    );
    assert_eq!(
        deliver(&view, Provider::ClaudeCode, "main", &touched).unwrap(),
        None
    );
}

#[test]
fn a_changed_file_is_delivered_again_marked_updated() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root rules");
    write(&worktree.join("src/CLAUDE.md"), "src rules");
    let touched = [view.join("api/src/lib.rs")];
    deliver(&view, Provider::ClaudeCode, "main", &touched).unwrap();
    write(&worktree.join("src/CLAUDE.md"), "src rules v2");
    assert_eq!(
        deliver(&view, Provider::ClaudeCode, "main", &touched).unwrap(),
        Some(format!(
            "UPDATED {}src rules v2",
            label(&view.join("api/src/CLAUDE.md"))
        ))
    );
}

#[test]
fn retargeting_the_repo_symlink_delivers_the_other_branch_file_as_updated() {
    let (_dir, view, worktree) = view_with_api();
    let feature = worktree.parent().unwrap().join("api-feature");
    write(&worktree.join("CLAUDE.md"), "main rules");
    write(&feature.join("CLAUDE.md"), "feat rules"); // same length on purpose
    let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for file in [worktree.join("CLAUDE.md"), feature.join("CLAUDE.md")] {
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }
    let touched = [view.join("api/lib.rs")];
    assert!(
        deliver(&view, Provider::ClaudeCode, "main", &touched)
            .unwrap()
            .unwrap()
            .ends_with("main rules")
    );
    fs::replace_symlink(&feature, &view.join("api")).unwrap();
    assert_eq!(
        deliver(&view, Provider::ClaudeCode, "main", &touched).unwrap(),
        Some(format!(
            "UPDATED {}feat rules",
            label(&view.join("api/CLAUDE.md"))
        ))
    );
}

#[test]
fn delivery_state_lives_in_the_config_dir_never_the_view_root() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    assert_eq!(
        state_dir(&view, Provider::ClaudeCode),
        view.join(".claude/ivar/instructions")
    );
    assert_eq!(
        state_dir(&view, Provider::Omp),
        view.join(".omp/ivar/instructions")
    );
    deliver(&view, Provider::ClaudeCode, "main", &[view.join("api/x")]).unwrap();
    assert_eq!(
        fs::read_dir(&view).unwrap(),
        [view.join(".claude"), view.join("api")]
    );
    let state: serde_json::Value = serde_json::from_str(
        &fs::read_text(&view.join(".claude/ivar/instructions/main.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(state.as_object().unwrap().len(), 1);
    assert_eq!(
        state[view.join("api/CLAUDE.md").as_str()],
        crate::infra::hash::text("root")[..16]
    );
}

#[test]
fn each_agent_key_has_its_own_delivery_state() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    let touched = [view.join("api/x")];
    for agent in ["main", "agent/7 x"] {
        assert!(
            deliver(&view, Provider::ClaudeCode, agent, &touched)
                .unwrap()
                .is_some(),
            "{agent}"
        );
    }
    for agent in ["main", "agent/7 x"] {
        assert_eq!(
            deliver(&view, Provider::ClaudeCode, agent, &touched).unwrap(),
            None,
            "{agent}"
        );
    }
    let dir = state_dir(&view, Provider::ClaudeCode);
    assert!(fs::is_file(&dir.join("main.json")).unwrap());
    assert!(fs::is_file(&dir.join("agent_7_x.json")).unwrap());
}

/// A view whose `api/CLAUDE.md` body is `body_chars` multibyte chars, and the
/// context `deliver` renders for it.
fn big_view(body_chars: usize) -> (tempfile::TempDir, Utf8PathBuf, String) {
    let (dir, view, worktree) = view_with_api();
    let body = "ü".repeat(body_chars);
    write(&worktree.join("CLAUDE.md"), &body);
    let context = format!("{}{body}", label(&view.join("api/CLAUDE.md")));
    (dir, view, context)
}

#[test]
fn slices_cut_on_char_boundaries_and_end_with_none() {
    let text = "é".repeat(CLAUDE_SLICE_CHARS) + "ab";
    assert_eq!(slice(&text, 0), Some("é".repeat(CLAUDE_SLICE_CHARS)));
    assert_eq!(slice(&text, 1).as_deref(), Some("ab"));
    assert_eq!(slice(&text, 2), None);
    assert_eq!(slice("", 0), None);
    assert_eq!(slice("short", 0).as_deref(), Some("short"));
}

#[test]
fn every_slice_of_one_call_reconstructs_one_computation() {
    // The label pushes this body into the last slice without exceeding the budget.
    let (_dir, view, context) = big_view((CLAUDE_SLICES - 1) * CLAUDE_SLICE_CHARS);
    let touched = [view.join("api/src/lib.rs")];
    let pieces: Vec<Option<String>> = (0..CLAUDE_SLICES)
        .map(|index| {
            deliver_slice(
                &view,
                Provider::ClaudeCode,
                "main",
                "toolu_1",
                &touched,
                index,
            )
            .unwrap()
        })
        .collect();
    assert!(pieces.iter().all(Option::is_some));
    // A second computation would have found the file already delivered and
    // returned nothing, so equality proves the other slices read slice 0's result.
    assert_eq!(pieces.into_iter().flatten().collect::<String>(), context);
    for index in 0..CLAUDE_SLICES {
        assert_eq!(
            deliver_slice(
                &view,
                Provider::ClaudeCode,
                "main",
                "toolu_2",
                &touched,
                index
            )
            .unwrap(),
            None,
            "{index}"
        );
    }
}

#[test]
fn concurrent_slice_entries_for_one_call_agree() {
    let (_dir, view, context) = big_view(3 * CLAUDE_SLICE_CHARS);
    let touched = [view.join("api/src/lib.rs")];
    let pieces: Vec<Option<String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..CLAUDE_SLICES)
            .map(|index| {
                let (view, touched) = (&view, &touched);
                scope.spawn(move || {
                    deliver_slice(
                        view,
                        Provider::ClaudeCode,
                        "main",
                        "toolu_c",
                        touched,
                        index,
                    )
                    .unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(pieces.into_iter().flatten().collect::<String>(), context);
}

#[test]
fn a_small_context_fills_only_the_first_slice() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    let touched = [view.join("api/x")];
    assert_eq!(
        deliver_slice(&view, Provider::ClaudeCode, "main", "toolu_s", &touched, 0).unwrap(),
        Some(format!("{}root", label(&view.join("api/CLAUDE.md"))))
    );
    for index in 1..CLAUDE_SLICES {
        assert_eq!(
            deliver_slice(
                &view,
                Provider::ClaudeCode,
                "main",
                "toolu_s",
                &touched,
                index
            )
            .unwrap(),
            None,
            "{index}"
        );
    }
}

#[test]
fn context_beyond_every_slice_names_only_the_cut_files_and_keeps_them_unseen() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    let big = "ü".repeat(CLAUDE_SLICES * CLAUDE_SLICE_CHARS);
    write(&worktree.join("src/CLAUDE.md"), &big);
    let touched = [view.join("api/src/x")];
    let root_file = view.join("api/CLAUDE.md");
    let src_file = view.join("api/src/CLAUDE.md");

    let pieces: Vec<String> = (0..=CLAUDE_SLICES)
        .filter_map(|index| {
            deliver_slice(
                &view,
                Provider::ClaudeCode,
                "main",
                "toolu_big",
                &touched,
                index,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(pieces.len(), CLAUDE_SLICES, "nothing past the last slice");
    let context: String = pieces.concat();
    assert_eq!(context.chars().count(), CLAUDE_SLICES * CLAUDE_SLICE_CHARS);
    let note = format!(
        "\n[The repository instructions continue beyond this message; read the rest of: {src_file}]"
    );
    assert!(pieces.last().unwrap().ends_with(&note), "{note}");
    assert!(context.starts_with(&format!(
        "{}root\n\n{}",
        label(&root_file),
        label(&src_file)
    )));

    // The root file fitted and is recorded; the cut file is not, so the
    // next call sends it again (not as UPDATED) and nothing else.
    let next = deliver_slice(
        &view,
        Provider::ClaudeCode,
        "main",
        "toolu_next",
        &touched,
        0,
    )
    .unwrap()
    .unwrap();
    assert!(
        next.starts_with(&label(&src_file)),
        "{}",
        next.chars().take(200).collect::<String>()
    );
    assert!(!next.contains(&label(&root_file)));
}

#[test]
fn a_call_touching_no_repo_writes_nothing() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    for index in 0..CLAUDE_SLICES {
        assert_eq!(
            deliver_slice(
                &view,
                Provider::ClaudeCode,
                "main",
                "toolu_x",
                &[Utf8PathBuf::from("/etc/hosts")],
                index
            )
            .unwrap(),
            None
        );
    }
    assert!(!fs::exists(&state_dir(&view, Provider::ClaudeCode)).unwrap());
}

#[test]
fn computing_a_call_prunes_call_files_older_than_ten_minutes() {
    let (_dir, view, worktree) = view_with_api();
    write(&worktree.join("CLAUDE.md"), "root");
    let dir = state_dir(&view, Provider::ClaudeCode);
    for name in ["old.ctx", "old.ctx.lock", "other.json", "fresh.ctx"] {
        write(&dir.join(name), "");
    }
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(11 * 60);
    for name in ["old.ctx", "old.ctx.lock", "other.json"] {
        std::fs::File::options()
            .write(true)
            .open(dir.join(name))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    deliver_slice(
        &view,
        Provider::ClaudeCode,
        "main",
        "toolu_new",
        &[view.join("api/x")],
        0,
    )
    .unwrap();
    assert_eq!(
        fs::read_dir(&dir).unwrap(),
        [
            dir.join("fresh.ctx"),
            dir.join("main.json"),
            dir.join("main.lock"),
            dir.join("other.json"),
            dir.join("toolu_new.ctx"),
            dir.join("toolu_new.ctx.lock"),
        ]
    );
}
