use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Attribute, Item, Path as SynPath, UseTree};

mod cmd {
    pub fn execute() {}
}

const DOMAIN_FOUNDATION_MODULES: &[&str] = &[
    "rules.rs",
    "plan.rs",
    "template.rs",
    "service_lifecycle.rs",
    "config_validation.rs",
];

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
fn domain_guard_ignores_comments_strings_lifetimes_and_nested_comments() {
    let source = r#"
        const DESCRIPTION: &str = "crate::watcher::WatchBackend";
        /* outer comment /* nested */ crate::watcher::WatchBackend still outer */
        #[cfg(test)]
        mod tests {
            use crate::watcher;
        }
        fn production(value: &'static str) {
            let _ = value;
            crate::plan::RunPlan::default();
        }
    "#;

    assert!(forbidden_dependency(source).is_none());
}

#[test]
fn domain_guard_ignores_raw_byte_strings() {
    let source = "const DESCRIPTION: &[u8] = br#\"crate::cmd::execute()\"#; fn production() { crate::plan::RunPlan::default(); }";

    assert!(forbidden_dependency(source).is_none());
}

#[test]
fn domain_guard_rejects_root_alias_qualified_reference() {
    for source in [
        "use crate as root; fn production() { root::cmd::execute(); }",
        "use super as parent; fn production() { parent::cmd::execute(); }",
    ] {
        let failure =
            forbidden_dependency(source).expect("root alias must resolve to crate or super");
        assert!(failure.contains("cmd"), "unexpected failure: {failure}");
    }
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
    let file = syn::parse_file(source).expect("boundary source must parse as Rust");
    let mut imports = ImportCollector::default();
    imports.visit_file(&file);
    if let Some(module) = imports.forbidden {
        return Some(format!("imports infrastructure module {module}"));
    }

    let mut paths = PathCollector {
        root_aliases: imports.root_aliases,
        forbidden: None,
    };
    paths.visit_file(&file);
    paths
        .forbidden
        .map(|module| format!("references infrastructure module {module}"))
}

#[derive(Default)]
struct ImportCollector {
    root_aliases: BTreeSet<String>,
    forbidden: Option<&'static str>,
}

impl<'ast> Visit<'ast> for ImportCollector {
    fn visit_item(&mut self, item: &'ast Item) {
        if !item_is_cfg_test(item) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        inspect_use_tree(&item.tree, false, self);
    }
}

struct PathCollector {
    root_aliases: BTreeSet<String>,
    forbidden: Option<&'static str>,
}

impl<'ast> Visit<'ast> for PathCollector {
    fn visit_item(&mut self, item: &'ast Item) {
        if !item_is_cfg_test(item) {
            visit::visit_item(self, item);
        }
    }

    fn visit_path(&mut self, path: &'ast SynPath) {
        if self.forbidden.is_none() {
            let mut segments = path.segments.iter();
            if let Some(first) = segments.next() {
                let rooted = is_root(&first.ident.to_string())
                    || self.root_aliases.contains(&first.ident.to_string());
                if rooted {
                    self.forbidden = segments
                        .map(|segment| segment.ident.to_string())
                        .find_map(|segment| infrastructure_module(&segment));
                }
            }
        }
        visit::visit_path(self, path);
    }
}

fn inspect_use_tree(tree: &UseTree, rooted: bool, collector: &mut ImportCollector) {
    if collector.forbidden.is_some() {
        return;
    }
    match tree {
        UseTree::Path(path) => {
            let name = path.ident.to_string();
            let rooted = rooted || is_root(&name);
            if rooted {
                collector.forbidden = infrastructure_module(&name);
            }
            inspect_use_tree(&path.tree, rooted, collector);
        }
        UseTree::Name(name) if rooted => {
            collector.forbidden = infrastructure_module(&name.ident.to_string());
        }
        UseTree::Rename(rename) => {
            let name = rename.ident.to_string();
            if is_root(&name) || (rooted && name == "self") {
                collector.root_aliases.insert(rename.rename.to_string());
            } else if rooted {
                collector.forbidden = infrastructure_module(&name);
            }
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                inspect_use_tree(tree, rooted, collector);
            }
        }
        UseTree::Glob(_) => {}
        UseTree::Name(_) => {}
    }
}

fn item_is_cfg_test(item: &Item) -> bool {
    let attrs = match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => return false,
        _ => return false,
    };
    attrs.iter().any(attribute_is_cfg_test)
}

fn attribute_is_cfg_test(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg")
        && attribute
            .meta
            .require_list()
            .is_ok_and(|list| list.tokens.to_string() == "test")
}

fn is_root(name: &str) -> bool {
    matches!(name, "crate" | "super")
}

fn infrastructure_module(name: &str) -> Option<&'static str> {
    INFRASTRUCTURE_MODULES
        .iter()
        .copied()
        .find(|module| *module == name)
}
