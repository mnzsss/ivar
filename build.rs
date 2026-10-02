use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let base = env!("CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=IVAR_RELEASE");
    let sha = if std::env::var_os("IVAR_RELEASE").is_some() {
        None
    } else {
        own_checkout_sha()
    };
    let version = sha.map_or_else(|| base.to_owned(), |sha| format!("{base}-dev+{sha}"));
    println!("cargo:rustc-env=IVAR_BUILD_VERSION={version}");
}

/// The short sha of this crate's own checkout, or `None` when the crate is
/// not the toplevel of a git worktree. A crates.io download has no checkout,
/// and the AUR source build unpacks inside the AUR package's clone; both are
/// releases and must not look like local builds.
fn own_checkout_sha() -> Option<String> {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);
    let toplevel = git(&manifest_dir, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&toplevel).canonicalize().ok()? != manifest_dir.canonicalize().ok()? {
        return None;
    }
    for path in [
        Some("HEAD".to_owned()),
        Some("packed-refs".to_owned()),
        git(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(file) = git(
            &manifest_dir,
            &["rev-parse", "--path-format=absolute", "--git-path", &path],
        )
        .filter(|file| Path::new(file).exists())
        {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    git(&manifest_dir, &["rev-parse", "--short", "HEAD"])
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}
