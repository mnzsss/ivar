#![allow(clippy::unwrap_used, clippy::expect_used)]

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn ivar_reaches_t3_only_through_its_public_binary() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let offenders: Vec<String> = files
        .iter()
        .filter(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            text.contains("Command::new(\"t3\")")
                || text.contains(".t3/")
                || text.contains("T3CODE_HOME")
        })
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "ivar must spawn `t3` through providers::t3::launch and never touch its data dir: {offenders:?}"
    );
}
