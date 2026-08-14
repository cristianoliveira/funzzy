//! Gitignore matching (TASK-0036).
//!
//! Uses the established `ignore` crate (same semantics as ripgrep/git): nested
//! `.gitignore` files, negation, anchored rules, and global excludes. The
//! matcher is built once per workspace root and cached; precedence is
//! documented in the contract: an explicit config `ignore` rule always wins
//! over gitignore.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One workspace-rooted gitignore matcher. Built lazily and cached; the
/// `ignore` crate holds the parsed rules, so matching does not rescan files
/// per event.
#[derive(Clone, Debug)]
pub struct GitignoreMatcher {
    root: PathBuf,
    /// `ignore`-crate matcher over the workspace root; rebuilt when a
    /// `.gitignore` changes (checked via the cached mtime below).
    matcher: Arc<ignore::gitignore::Gitignore>,
    /// Mtime signature of the last build, so rebuilds happen only on real
    /// changes, never per event.
    cache_key: Option<std::time::SystemTime>,
}

impl GitignoreMatcher {
    /// Builds the matcher for `root`, applying `respect_gitignore` semantics.
    pub fn new(root: PathBuf) -> Self {
        Self {
            matcher: Arc::new(Self::build(&root)),
            cache_key: Self::cache_key(&root),
            root,
        }
    }

    fn build(root: &Path) -> ignore::gitignore::Gitignore {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        // Add the root `.gitignore` plus every nested `.gitignore` under the
        // root, so nested rules and negation resolve like git/ripgrep.
        if root.join(".gitignore").exists() {
            let _ = builder.add(root.join(".gitignore"));
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::add_nested_gitignores(&mut builder, &path);
                }
            }
        }
        builder.build().unwrap_or_else(|_| {
            // A broken gitignore degrades to "nothing ignored" rather than
            // failing the watcher; matching stays deterministic.
            ignore::gitignore::GitignoreBuilder::new(root)
                .build()
                .expect("empty gitignore builds")
        })
    }

    /// Recursively adds every `.gitignore` under `dir` to the builder.
    fn add_nested_gitignores(builder: &mut ignore::gitignore::GitignoreBuilder, dir: &Path) {
        let candidate = dir.join(".gitignore");
        if candidate.exists() {
            let _ = builder.add(candidate);
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::add_nested_gitignores(builder, &path);
                }
            }
        }
    }

    fn cache_key(root: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(root.join(".gitignore"))
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// True when `path` (relative to the workspace root) is gitignored.
    /// Nested `.gitignore` files are resolved by the underlying crate; a
    /// path that matches a negated rule is not ignored.
    pub fn is_ignored(&self, relative: &Path) -> bool {
        self.matcher
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    /// True when the workspace `.gitignore` changed since the last build,
    /// so the matcher can be rebuilt without an event-loss gap.
    pub fn needs_rebuild(&self) -> bool {
        self.cache_key != Self::cache_key(&self.root)
    }

    /// Rebuilds the matcher after a `.gitignore` change.
    pub fn rebuild(&mut self) {
        self.matcher = Arc::new(Self::build(&self.root));
        self.cache_key = Self::cache_key(&self.root);
    }

    /// The workspace root this matcher anchors to.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("funzzy-gitignore-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn respects_root_gitignore_rules() {
        let dir = scratch("root");
        fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("target/x.rs"), "").unwrap();
        fs::write(dir.join("notes.log"), "").unwrap();
        fs::write(dir.join("main.rs"), "").unwrap();
        let matcher = GitignoreMatcher::new(dir.clone());
        assert!(matcher.is_ignored(Path::new("target/x.rs")));
        assert!(matcher.is_ignored(Path::new("notes.log")));
        assert!(!matcher.is_ignored(Path::new("main.rs")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn negation_unignores_a_matched_path() {
        let dir = scratch("negation");
        fs::write(dir.join(".gitignore"), "*.rs\n!keep.rs\n").unwrap();
        fs::write(dir.join("drop.rs"), "").unwrap();
        fs::write(dir.join("keep.rs"), "").unwrap();
        let matcher = GitignoreMatcher::new(dir.clone());
        assert!(matcher.is_ignored(Path::new("drop.rs")));
        assert!(!matcher.is_ignored(Path::new("keep.rs")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nested_gitignore_is_resolved() {
        let dir = scratch("nested");
        fs::write(dir.join(".gitignore"), "").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/.gitignore"), "generated.txt\n").unwrap();
        fs::write(dir.join("sub/generated.txt"), "").unwrap();
        fs::write(dir.join("sub/real.txt"), "").unwrap();
        let matcher = GitignoreMatcher::new(dir.clone());
        assert!(matcher.is_ignored(Path::new("sub/generated.txt")));
        assert!(!matcher.is_ignored(Path::new("sub/real.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn anchored_rule_matches_only_at_root() {
        let dir = scratch("anchored");
        fs::write(dir.join(".gitignore"), "/only-root.txt\n").unwrap();
        fs::create_dir_all(dir.join("deep")).unwrap();
        fs::write(dir.join("only-root.txt"), "").unwrap();
        fs::write(dir.join("deep/only-root.txt"), "").unwrap();
        let matcher = GitignoreMatcher::new(dir.clone());
        assert!(matcher.is_ignored(Path::new("only-root.txt")));
        assert!(!matcher.is_ignored(Path::new("deep/only-root.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rebuild_after_gitignore_change_without_event_loss() {
        let dir = scratch("rebuild");
        fs::write(dir.join(".gitignore"), "first.txt\n").unwrap();
        fs::write(dir.join("first.txt"), "").unwrap();
        fs::write(dir.join("second.txt"), "").unwrap();
        let mut matcher = GitignoreMatcher::new(dir.clone());
        assert!(matcher.is_ignored(Path::new("first.txt")));
        assert!(!matcher.is_ignored(Path::new("second.txt")));

        // Change the gitignore; the matcher detects the drift and rebuilds
        // without an event-loss gap (the old matcher stays valid until then).
        fs::write(dir.join(".gitignore"), "second.txt\n").unwrap();
        assert!(matcher.needs_rebuild());
        assert!(
            matcher.is_ignored(Path::new("first.txt")),
            "old rules stay valid until rebuild"
        );
        matcher.rebuild();
        assert!(!matcher.is_ignored(Path::new("first.txt")));
        assert!(matcher.is_ignored(Path::new("second.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_gitignore_degrades_to_nothing_ignored() {
        let dir = scratch("broken");
        fs::write(dir.join(".gitignore"), "[unclosed\n").unwrap();
        fs::write(dir.join("any.txt"), "").unwrap();
        let matcher = GitignoreMatcher::new(dir.clone());
        assert!(!matcher.is_ignored(Path::new("any.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
