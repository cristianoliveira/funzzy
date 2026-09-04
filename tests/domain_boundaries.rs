use std::path::{Path, PathBuf};

mod cmd {
    pub fn execute() {}
}

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
fn every_domain_rust_file_avoids_infrastructure_dependencies() {
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
        assert_domain_dependencies_are_isolated(&file, &source);
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
        let failure = forbidden_dependency(source).expect("mutation must be rejected");
        assert!(failure.contains("watcher"), "unexpected failure: {failure}");
    }
}

#[test]
fn domain_guard_rejects_compiling_qualified_crate_reference() {
    crate::cmd::execute();

    let failure = forbidden_dependency(include_str!("domain_boundaries.rs"))
        .expect("qualified crate reference must be rejected");
    assert!(failure.contains("cmd"), "unexpected failure: {failure}");
}

#[test]
fn domain_guard_continues_after_a_cfg_test_item() {
    let source = "#[cfg(test)] mod tests { use crate::watcher; }\nfn production() { crate::cmd::execute(); }";

    let failure = forbidden_dependency(source).expect("production code after cfg(test) must scan");
    assert!(failure.contains("cmd"), "unexpected failure: {failure}");
}

#[test]
fn domain_guard_skips_comments_strings_and_only_the_cfg_test_item() {
    let source = r#"
        const DESCRIPTION: &str = "crate::watcher::WatchBackend";
        // crate::cmd::execute();
        #[cfg(test)]
        mod tests {
            use crate::watcher;
        }
        fn production() {
            crate::plan::RunPlan::default();
        }
    "#;

    assert!(forbidden_dependency(source).is_none());
}

#[test]
fn domain_guard_rejects_qualified_path_after_a_lifetime() {
    let source = "fn production(value: &'static str) { let _ = value; crate::cmd::execute(); }";

    let failure = forbidden_dependency(source).expect("lifetime must not hide a qualified path");
    assert!(failure.contains("cmd"), "unexpected failure: {failure}");
}

#[test]
fn domain_guard_ignores_nested_block_comments() {
    let source = "/* outer comment /* nested */ crate::watcher::WatchBackend still outer */ fn production() { crate::plan::RunPlan::default(); }";

    assert!(forbidden_dependency(source).is_none());
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

fn assert_domain_dependencies_are_isolated(file: &Path, source: &str) {
    if let Some(failure) = forbidden_dependency(source) {
        panic!("domain module {} {failure}", file.display());
    }
}

fn forbidden_dependency(source: &str) -> Option<String> {
    let tokens = production_tokens(source);

    if let Some(module) = forbidden_module_in_imports(&tokens) {
        return Some(format!("imports infrastructure module {module}"));
    }
    forbidden_module_in_qualified_paths(&tokens)
        .map(|module| format!("references infrastructure module {module}"))
}

fn forbidden_module_in_imports(tokens: &[String]) -> Option<&'static str> {
    let mut start = 0;
    while let Some(use_index) = tokens[start..].iter().position(|token| token == "use") {
        let use_index = start + use_index;
        let end = tokens[use_index..]
            .iter()
            .position(|token| token == ";")
            .map(|offset| use_index + offset)
            .unwrap_or(tokens.len());
        let statement = &tokens[use_index..end];
        if statement
            .iter()
            .any(|token| matches!(token.as_str(), "crate" | "super"))
        {
            if let Some(module) = forbidden_module(statement) {
                return Some(module);
            }
        }
        start = end.saturating_add(1);
    }
    None
}

fn forbidden_module_in_qualified_paths(tokens: &[String]) -> Option<&'static str> {
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.as_str(), "crate" | "super") {
            continue;
        }
        let mut path = Vec::new();
        for token in &tokens[index + 1..] {
            if matches!(token.as_str(), "::" | "super") {
                continue;
            }
            if is_identifier(token) {
                path.push(token.clone());
                continue;
            }
            break;
        }
        if let Some(module) = forbidden_module(&path) {
            return Some(module);
        }
    }
    None
}

fn forbidden_module(tokens: &[String]) -> Option<&'static str> {
    INFRASTRUCTURE_MODULES
        .iter()
        .copied()
        .find(|module| tokens.iter().any(|token| token == module))
}

fn is_identifier(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

fn production_tokens(source: &str) -> Vec<String> {
    let tokens = tokens_without_comments_or_strings(source);
    let mut production = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        if tokens[index..].starts_with(&[
            "#".to_owned(),
            "[".to_owned(),
            "cfg".to_owned(),
            "(".to_owned(),
            "test".to_owned(),
            ")".to_owned(),
            "]".to_owned(),
        ]) {
            index = skip_cfg_test_item(&tokens, index + 7);
        } else {
            production.push(tokens[index].clone());
            index += 1;
        }
    }
    production
}

fn skip_cfg_test_item(tokens: &[String], mut index: usize) -> usize {
    while index < tokens.len() && !matches!(tokens[index].as_str(), "{" | ";") {
        index += 1;
    }
    if index == tokens.len() || tokens[index] == ";" {
        return index.saturating_add(1);
    }

    let mut depth = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    tokens.len()
}

fn tokens_without_comments_or_strings(source: &str) -> Vec<String> {
    let characters: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        match characters[index] {
            '/' if characters.get(index + 1) == Some(&'/') => {
                index += 2;
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
            }
            '/' if characters.get(index + 1) == Some(&'*') => {
                index = skip_nested_block_comment(&characters, index + 2);
            }
            '"' => index = skip_quoted(&characters, index),
            '\'' if char_literal_end(&characters, index).is_some() => {
                index = char_literal_end(&characters, index).expect("checked above");
            }
            'r' if raw_string_end(&characters, index).is_some() => {
                index = raw_string_end(&characters, index).expect("checked above");
            }
            character if character == '_' || character.is_ascii_alphanumeric() => {
                let start = index;
                index += 1;
                while index < characters.len()
                    && (characters[index] == '_' || characters[index].is_ascii_alphanumeric())
                {
                    index += 1;
                }
                tokens.push(characters[start..index].iter().collect());
            }
            ':' if characters.get(index + 1) == Some(&':') => {
                tokens.push("::".to_owned());
                index += 2;
            }
            character if matches!(character, '#' | '[' | ']' | '(' | ')' | '{' | '}' | ';') => {
                tokens.push(character.to_string());
                index += 1;
            }
            _ => index += 1,
        }
    }
    tokens
}

fn skip_nested_block_comment(characters: &[char], mut index: usize) -> usize {
    let mut depth = 1;
    while index < characters.len() {
        match (characters.get(index), characters.get(index + 1)) {
            (Some('/'), Some('*')) => {
                depth += 1;
                index += 2;
            }
            (Some('*'), Some('/')) => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    characters.len()
}

fn char_literal_end(characters: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    if characters.get(index) == Some(&'\\') {
        index += 2;
    } else {
        index += 1;
    }
    (characters.get(index) == Some(&'\'')).then_some(index + 1)
}

fn skip_quoted(characters: &[char], mut index: usize) -> usize {
    let quote = characters[index];
    index += 1;
    while index < characters.len() {
        if characters[index] == '\\' {
            index += 2;
        } else if characters[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    characters.len()
}

fn raw_string_end(characters: &[char], start: usize) -> Option<usize> {
    let mut quote = start + 1;
    while characters.get(quote) == Some(&'#') {
        quote += 1;
    }
    if characters.get(quote) != Some(&'"') {
        return None;
    }
    let hashes = quote - start - 1;
    let mut index = quote + 1;
    while index < characters.len() {
        if characters[index] == '"'
            && characters[index + 1..]
                .iter()
                .take(hashes)
                .all(|char| *char == '#')
            && index + hashes < characters.len()
        {
            return Some(index + hashes + 1);
        }
        index += 1;
    }
    Some(characters.len())
}
