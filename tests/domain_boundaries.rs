use std::path::{Path, PathBuf};

const DOMAIN_FOUNDATION_MODULES: &[&str] =
    &["rules.rs", "plan.rs", "template.rs", "service_lifecycle.rs"];

const INFRASTRUCTURE_MODULES: &[&str] = &[
    "app",
    "arguments",
    "cli",
    "cmd",
    "config",
    "control",
    "control_client",
    "diagnostics",
    "event_stream",
    "logging",
    "stdout",
    "watch_loop",
    "watcher",
    "watcher_state",
    "workers",
    "workflow",
];

#[test]
fn every_domain_rust_file_avoids_infrastructure_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let domain_root = root.join("src/domain");
    let mut files = rust_files_under(&domain_root);
    assert!(
        !files.is_empty(),
        "src/domain must contain the boundary module"
    );
    files.extend(
        DOMAIN_FOUNDATION_MODULES
            .iter()
            .map(|module| root.join("src").join(module)),
    );
    files.sort();
    files.dedup();

    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", file.display()));
        assert_domain_imports_are_isolated(&file, &source);
    }
}

#[test]
fn domain_guard_rejects_direct_aliased_grouped_multiline_and_super_imports() {
    for source in [
        "use crate::watcher::WatchBackend;",
        "use crate::watcher as runtime_watcher;",
        "use crate::{\n    plan::RunPlan,\n    watcher::WatchBackend,\n};",
        "use crate::{\n    watcher as runtime_watcher,\n    plan::RunPlan,\n};",
        "use super::super::watcher::WatchBackend;",
    ] {
        let failure = forbidden_import(source).expect("mutation must be rejected");
        assert!(failure.contains("watcher"), "unexpected failure: {failure}");
    }
}

#[test]
fn domain_guard_ignores_infrastructure_mentions_after_test_boundary() {
    let source = "use crate::plan::RunPlan;\n#[cfg(test)]\nmod tests { use crate::watcher; }";

    assert!(forbidden_import(source).is_none());
}

#[test]
fn domain_boundary_does_not_publish_speculative_ports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();

    assert!(lib.contains("mod domain;"));
    assert!(!lib.contains("pub mod domain;"));
    assert!(
        !root.join("src/domain/ports.rs").exists(),
        "a port becomes public only with its first consumer/adapter"
    );
}

fn rust_files_under(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let path = entry.expect("failed to read directory entry").path();
        if path.is_dir() {
            files.extend(rust_files_under(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn assert_domain_imports_are_isolated(file: &Path, source: &str) {
    if let Some(failure) = forbidden_import(source) {
        panic!("domain module {} {failure}", file.display());
    }
}

fn forbidden_import(source: &str) -> Option<String> {
    let production_source = source.split("#[cfg(test)]").next().unwrap();
    let tokens = tokens(production_source);
    let mut start = 0;

    while let Some(use_index) = tokens[start..].iter().position(|token| token == "use") {
        let use_index = start + use_index;
        let end = tokens[use_index..]
            .iter()
            .position(|token| token == ";")
            .map(|offset| use_index + offset)
            .unwrap_or(tokens.len());
        let statement = &tokens[use_index..end];
        let imports_from_domain_tree = statement
            .iter()
            .any(|token| matches!(token.as_str(), "crate" | "super"));

        if imports_from_domain_tree {
            if let Some(infrastructure) = INFRASTRUCTURE_MODULES
                .iter()
                .find(|module| statement.iter().any(|token| token == **module))
            {
                return Some(format!("imports infrastructure module {infrastructure}"));
            }
        }
        start = end.saturating_add(1);
    }
    None
}

fn tokens(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();

    for character in source.chars() {
        if character == '_' || character.is_ascii_alphanumeric() {
            word.push(character);
        } else {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            if character == ';' {
                tokens.push(";".to_owned());
            }
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
}
