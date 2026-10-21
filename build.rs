use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn watch_git_path(root: &Path, path: &str) {
    if let Some(path) = git(root, &["rev-parse", "--git-path", path]) {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn source_commit(root: &Path) -> Option<String> {
    let repository = git(root, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&repository).canonicalize().ok()? != root.canonicalize().ok()? {
        return None;
    }

    watch_git_path(root, "HEAD");
    if let Some(branch) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(root, &branch);
    }
    watch_git_path(root, "packed-refs");
    git(root, &["rev-parse", "--short=12", "HEAD"])
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version = env!("CARGO_PKG_VERSION");
    let version = match source_commit(root) {
        Some(commit) => format!("{version} ({commit})"),
        None => version.to_owned(),
    };
    println!("cargo:rustc-env=FUNZZY_BUILD_VERSION={version}");
}
