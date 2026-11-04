use camino::{Utf8Path, Utf8PathBuf};

use crate::common::{git, hall_root, seeded_repo};
use crate::support::run_ok;

pub(crate) fn web_instructions(command: &str, token: &str) -> String {
    format!(
        "# web\n\n- The ONLY supported way to run web locally is `{command}`. Cite [src:{token}] \
         when you give it.\n"
    )
}

pub(crate) fn rule(title: &str, what: &str, command: &str, token: &str) -> String {
    format!(
        "# {title}\n\n- The ONLY supported command to {what} is `{command}`. Every answer about \
         it must cite [src:{token}].\n"
    )
}

/// A >24,000-char instruction file whose one relevant fact sits near the end.
pub(crate) fn deep_instructions() -> String {
    let mut lines = vec!["# backend".to_owned(), String::new()];
    let mut n = 0;
    while lines.iter().map(|line| line.len() + 1).sum::<usize>() < 24_500 {
        n += 1;
        lines.push(format!(
            "- Rule {n}: keep module {n} free of hidden side effects, cover it with a focused \
             test, and describe its public API in the module header."
        ));
    }
    let near_end = lines.len() - 2;
    lines.insert(
        near_end,
        "- Restart the local services ONLY with `npm run serve:zulu` and cite [src:DEEP-88]."
            .to_owned(),
    );
    let text = lines.join("\n") + "\n";
    assert!(text.chars().count() > 24_000);
    text
}

/// A committed repo at `origins/<name>` holding `files` (relative path, content).
pub(crate) fn origin(root: &Utf8Path, name: &str, files: &[(&str, String)]) -> Utf8PathBuf {
    let path = seeded_repo(&root.parent().unwrap().join("origins").join(name), "main");
    for (relative, body) in files {
        let file = path.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, body).unwrap();
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "fixture"]);
    path
}

/// A synced hall declaring claude-code, omp and opencode, with `web` (promotion), `app`
/// (nested file, Bash) and `backend` (deep file), all on `main`.
pub(crate) fn live_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    run_ok(&root, &["init"]);
    let main_web = web_instructions("pnpm serve:alpha", "MAIN-41");
    let web = origin(
        &root,
        "web",
        &[
            ("CLAUDE.md", main_web.clone()),
            ("AGENTS.md", main_web),
            ("src/index.ts", "export const web = 1;\n".to_owned()),
        ],
    );
    let app_root = rule("app", "install the app", "pnpm boot:kilo", "ROOT-11");
    let app = origin(
        &root,
        "app",
        &[
            ("CLAUDE.md", app_root.clone()),
            ("AGENTS.md", app_root),
            (
                "packages/api/CLAUDE.md",
                rule("api", "run the api tests", "pnpm probe:lima", "API-22"),
            ),
            (
                "packages/api/src/x.ts",
                "export const api = 1;\n".to_owned(),
            ),
        ],
    );
    let backend = origin(
        &root,
        "backend",
        &[
            ("CLAUDE.md", deep_instructions()),
            ("src/index.ts", "export const backend = 1;\n".to_owned()),
        ],
    );
    let repos = [("web", &web), ("app", &app), ("backend", &backend)]
        .iter()
        .map(|(name, url)| format!(r#"{{"default_branch":"main","name":"{name}","url":"{url}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":["claude-code","omp","opencode"],"default":"claude-code"}},"repos":[{repos}],"version":1}}"#
        ),
    )
    .unwrap();
    run_ok(&root, &["sync"]);
    (guard, root)
}

/// Give every worktree its owner write bit back so the TempDir can be removed.
pub(crate) fn unguard(root: &Utf8Path) {
    use std::os::unix::fs::PermissionsExt;
    for repo in std::fs::read_dir(root.join(".ivar/repos"))
        .unwrap()
        .flatten()
    {
        for worktree in std::fs::read_dir(repo.path()).unwrap().flatten() {
            let path = worktree.path();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o200)).unwrap();
        }
    }
}
