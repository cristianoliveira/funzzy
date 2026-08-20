extern crate yaml_rust2;

use crate::errors::{FzzError, Result};

use self::yaml_rust2::Yaml;
use std::collections::BTreeMap;

pub fn extract_list(yaml: &Yaml, prop: &str) -> Result<Vec<String>> {
    match &yaml[prop] {
        Yaml::Array(ref items) => Ok(items
            .iter()
            .map(|i| String::from(i.as_str().unwrap_or("_invalid_value_")))
            .collect()),
        Yaml::String(ref item) => Ok(vec![String::from(item.as_str())]),
        Yaml::BadValue => Err(FzzError::InvalidConfigError(
            format!(
                "Missing '{}' in rule\n```yaml\n{}\n```",
                prop,
                yaml_to_string(yaml, 0),
            ),
            None,
            Some("Check for typos or wrong identation".to_string()),
        )),
        unknown => Err(FzzError::InvalidConfigError(
            format!(
                "Invalid property '{}' in rule below
Expected a list (Array) but got: {}
```yaml
{}
```",
                prop,
                get_type(unknown),
                yaml_to_string(yaml, 0),
            ),
            None,
            Some(
                "Check if the property is defined, with the right type and identation".to_string(),
            ),
        )),
    }
}

pub fn get_type(yaml: &Yaml) -> String {
    match yaml {
        Yaml::Hash(_) => "Hash".to_string(),
        Yaml::Array(_) => "Array".to_string(),
        Yaml::String(_) => "String".to_string(),
        Yaml::Boolean(_) => "Boolean".to_string(),
        Yaml::Integer(_) => "Integer".to_string(),
        Yaml::Real(_) => "Real".to_string(),
        _ => "Unknown".to_string(),
    }
}

pub fn extract_string(yaml: &Yaml, prop: &str) -> Result<String> {
    match &yaml[prop] {
        Yaml::String(ref item) => Ok(String::from(item.as_str())),
        Yaml::BadValue => Err(FzzError::InvalidConfigError(
            format!(
                "Missing '{}' in rule\n```yaml\n{}\n```",
                prop,
                yaml_to_string(yaml, 0),
            ),
            None,
            Some("Check for typos or wrong identation".to_string()),
        )),
        unknown => Err(FzzError::InvalidConfigError(
            format!(
                "Invalid property '{}' in rule below
Expected 'String' but got: {:?}
```
{}
```",
                prop,
                get_type(unknown),
                yaml_to_string(yaml, 0),
            ),
            None,
            Some(
                "Check if the property is defined, with the right type and identation".to_string(),
            ),
        )),
    }
}

/// Extracts an optional string-to-string map. Missing values yield an empty
/// map; names must be non-empty strings and values must be strings.
pub fn extract_optional_string_map(yaml: &Yaml, prop: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    match &yaml[prop] {
        Yaml::BadValue => Ok(values),
        Yaml::Hash(items) => {
            for (name, value) in items {
                let Some(name) = name.as_str() else {
                    return Err(FzzError::InvalidConfigError(
                        format!("Property '{}' environment names must be strings", prop),
                        None,
                        None,
                    ));
                };
                if name.trim().is_empty() {
                    return Err(FzzError::InvalidConfigError(
                        "Environment variable name cannot be empty".to_owned(),
                        None,
                        None,
                    ));
                }
                let Some(value) = value.as_str() else {
                    return Err(FzzError::InvalidConfigError(
                        format!("Environment value for '{}' must be a string", name),
                        None,
                        None,
                    ));
                };
                values.insert(name.to_owned(), value.to_owned());
            }
            Ok(values)
        }
        unknown => Err(FzzError::InvalidConfigError(
            format!(
                "Property '{}' must be a string-to-string object, got {}",
                prop,
                get_type(unknown)
            ),
            None,
            None,
        )),
    }
}

/// Extracts an optional string property. Missing values yield `Ok(None)`;
/// present non-string values are errors. Empty strings are rejected so an
/// empty `parallel` group name cannot silently disable grouping.
pub fn extract_optional_string(yaml: &Yaml, prop: &str) -> Result<Option<String>> {
    match &yaml[prop] {
        Yaml::BadValue => Ok(None),
        Yaml::String(ref item) => {
            let value = item.as_str();
            if value.trim().is_empty() {
                Err(FzzError::InvalidConfigError(
                    format!(
                        "Property '{}' cannot be empty\n```yaml\n{}\n```",
                        prop,
                        yaml_to_string(yaml, 0),
                    ),
                    None,
                    Some("Provide a non-empty value".to_string()),
                ))
            } else {
                Ok(Some(String::from(value)))
            }
        }
        unknown => Err(FzzError::InvalidConfigError(
            format!(
                "Invalid property '{}' in rule below
Expected 'String' but got: {}
```
{}
```",
                prop,
                get_type(unknown),
                yaml_to_string(yaml, 0),
            ),
            None,
            Some(
                "Check if the property is defined, with the right type and identation".to_string(),
            ),
        )),
    }
}

pub fn extract_bool(yaml: &Yaml, prop: &str) -> bool {
    match yaml[prop] {
        Yaml::Boolean(item) => item,
        _ => false,
    }
}

pub fn yaml_to_string(yaml: &Yaml, identation: u8) -> String {
    let spaces = " ".repeat((identation * 2).into());
    let next_identation: u8 = identation + 1;
    match yaml {
        Yaml::Hash(hash) => {
            let mut result = String::new();
            for (key, value) in hash {
                if let Yaml::Hash(_) | Yaml::Array(_) = value {
                    result.push_str(&format!(
                        "{}{}:\n{}",
                        spaces,
                        yaml_to_string(key, next_identation),
                        yaml_to_string(value, next_identation)
                    ));
                    continue;
                }
                result.push_str(&format!(
                    "{}{}: {}\n",
                    spaces,
                    yaml_to_string(key, next_identation),
                    yaml_to_string(value, next_identation)
                ));
            }

            if let Some(without_return) = result.strip_suffix("\n") {
                without_return.to_string()
            } else {
                result
            }
        }
        Yaml::Array(items) => {
            let mut result = String::new();
            for item in items {
                if let Yaml::Hash(_) = item {
                    let hash_str = yaml_to_string(item, 0);
                    let hash_lines = hash_str.split("\n").collect::<Vec<&str>>();
                    let first_line = hash_lines.first().unwrap_or(&"");
                    let hash_same_identation = hash_lines
                        .iter()
                        .skip(1)
                        .map(|line| format!("  {}{}", spaces, line))
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<String>>()
                        .join("\n");

                    if hash_same_identation.is_empty() {
                        result.push_str(&format!("{}- {}\n", spaces, first_line));
                        continue;
                    }

                    result.push_str(&format!(
                        "{}- {}\n{}\n",
                        spaces, first_line, hash_same_identation
                    ));
                } else {
                    result.push_str(&format!(
                        "{}- {}\n",
                        spaces,
                        yaml_to_string(item, identation)
                    ));
                }
            }

            result
        }
        Yaml::String(item) => item.to_string(),
        Yaml::Boolean(item) => item.to_string(),
        Yaml::Integer(item) => item.to_string(),
        Yaml::Real(item) => item.to_string(),
        unknown => format!("{:?}", unknown),
    }
}

#[cfg(test)]
mod tests {
    use self::yaml_rust2::YamlLoader;
    use super::*;
    fn clean_yaml_str(yaml_str: &str) -> String {
        yaml_str
            .split("\n")
            .map(|line| line.trim())
            .filter(|line| !line.starts_with("#") && !line.is_empty())
            .collect::<Vec<&str>>()
            .join("\n")
    }

    #[test]
    fn parses_yaml_to_yaml_instance_to_string_back() {
        let og_yaml_str = "# Initial YAML
        - foo: bar
          run: echo foo
          run_on_init: true

        - name: aaaaaaaa
          run: echo ooooooooo
          integer: 190
          real: 1.90
          run_on_init: true

        - foo: fooooo
          run: echo aaaaa
          run_on_init: true
          ";

        let docs = YamlLoader::load_from_str(og_yaml_str).unwrap();
        let yaml_str = yaml_to_string(&docs[0], 0);

        assert_eq!(clean_yaml_str(og_yaml_str), clean_yaml_str(&yaml_str));
    }

    #[test]
    fn fails_when_attempt_to_extract_list_from_nonlist_yaml() {
        let og_yaml_str = "# Initial YAML
fooobar:
    run: echo foo
    run_on_init: true
    ahashlist:
        - one: 1
        - two: zwei
    alist:
        - bar
        - baz";

        let docs = YamlLoader::load_from_str(og_yaml_str).unwrap();
        match extract_list(&docs[0], "fooobar") {
            Ok(_) => panic!("Failed to fail extracting list from non-list yaml"),
            Err(err) => {
                assert_eq!(
                    format!("{}", err),
                    "Invalid property 'fooobar' in rule below
Expected a list (Array) but got: Hash
```yaml
fooobar:
  run: echo foo
  run_on_init: true
  ahashlist:
    - one: 1
    - two: zwei
  alist:
    - bar
    - baz
```
Hint: Check if the property is defined, with the right type and identation",
                );
            }
        }
    }
}

#[cfg(test)]
mod characterization_tests {
    //! Parser-boundary characterization for the accepted YAML dialect
    //! (TASK-0112). These tests pin current behavior — anchors, block
    //! scalars, quoting, nulls, duplicate keys, scalar typing quirks,
    //! multi-document input, and scan-error surface — so a parser swap
    //! surfaces every difference instead of hiding it.
    use super::*;
    use yaml_rust2::YamlLoader;

    fn parse_one(src: &str) -> Yaml {
        let docs = YamlLoader::load_from_str(src).expect("valid yaml under test");
        assert_eq!(docs.len(), 1, "expected exactly one document: {src:?}");
        docs.into_iter().next().unwrap()
    }

    #[test]
    fn substitutes_scalar_hash_and_array_anchors() {
        let doc = parse_one(
            "a: &x hello\nrun: *x\nbase: &b\n  change: src\nmap: *b\nlist: &l\n  - one\nref: *l",
        );
        assert_eq!(doc["run"].as_str(), Some("hello"));
        assert_eq!(doc["map"]["change"].as_str(), Some("src"));
        assert_eq!(doc["ref"].as_vec().unwrap().len(), 1);
    }

    #[test]
    fn merge_key_is_kept_as_literal_not_merged() {
        let doc = parse_one("defaults: &d\n  change: c1\njobs:\n  - <<: *d\n    run: r");
        let job = &doc["jobs"][0];
        // Merge is not performed: `<<` survives as a plain key.
        assert_eq!(job["<<"]["change"].as_str(), Some("c1"));
        assert_eq!(job["run"].as_str(), Some("r"));
        assert_eq!(job["change"], Yaml::BadValue);
    }

    #[test]
    fn literal_block_scalar_preserves_lines_and_trailing_newline() {
        // TASK-0112: yaml-rust2 applies YAML 1.2 clip chomping, keeping the
        // single trailing newline (yaml-rust 0.4 dropped it).
        let doc = parse_one("run: |\n  echo one\n  echo two");
        assert_eq!(doc["run"].as_str(), Some("echo one\necho two\n"));

        let doc = parse_one("run: |\n  echo one\n");
        assert_eq!(doc["run"].as_str(), Some("echo one\n"));
    }

    #[test]
    fn folded_block_scalar_joins_lines_with_spaces() {
        // Trailing newline kept under clip chomping (TASK-0112 delta).
        let doc = parse_one("run: >\n  echo one\n  two");
        assert_eq!(doc["run"].as_str(), Some("echo one two\n"));
    }

    #[test]
    fn quoting_rules_for_single_and_double_quoted_scalars() {
        let doc = parse_one("run: 'echo ''quoted'''\ntitle: \"line1\\nline2\"");
        assert_eq!(doc["run"].as_str(), Some("echo 'quoted'"));
        assert_eq!(doc["title"].as_str(), Some("line1\nline2"));
    }

    #[test]
    fn null_forms_parse_to_null_and_render_as_null() {
        for src in ["name: ~", "name:", "name: null"] {
            let doc = parse_one(src);
            assert_eq!(doc["name"], Yaml::Null, "for {src:?}");
        }
        assert_eq!(yaml_to_string(&parse_one("name: ~"), 0), "name: Null");
    }

    #[test]
    fn null_run_is_rejected_as_unknown_type_not_silent_empty() {
        let doc = parse_one("run: ~");
        let err = extract_string(&doc, "run").expect_err("Null run must be rejected");
        assert!(
            err.to_string().contains("Expected 'String'"),
            "typed error, got: {err}"
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_not_last_wins() {
        // TASK-0112 delta: yaml-rust 0.4 silently kept the last duplicate
        // key; yaml-rust2 rejects duplicates as a scan error (YAML 1.2).
        let err = YamlLoader::load_from_str("name: a\nname: b").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("duplicated key in mapping") && text.contains("line 2"),
            "duplicate key error names construct and position, got: {text}"
        );
    }

    #[test]
    fn boolean_and_null_case_variants_follow_yaml_12_core_schema() {
        // TASK-0112 delta: `True`/`TRUE`/`False` now parse as booleans
        // (yaml-rust treated them as strings); `Null`/`NULL` now parse as
        // strings while lowercase `null` stays null.
        let doc = parse_one("a: true\nb: True\nc: TRUE\nd: False\ne: on\nf: yes");
        assert!(matches!(doc["a"], Yaml::Boolean(true)));
        assert!(matches!(doc["b"], Yaml::Boolean(true)));
        assert!(matches!(doc["c"], Yaml::Boolean(true)));
        assert!(matches!(doc["d"], Yaml::Boolean(false)));
        assert_eq!(doc["e"].as_str(), Some("on"));
        assert_eq!(doc["f"].as_str(), Some("yes"));

        let doc = parse_one("a: null\nb: Null\nc: NULL");
        assert_eq!(doc["a"], Yaml::Null);
        assert_eq!(doc["b"].as_str(), Some("Null"));
        assert_eq!(doc["c"].as_str(), Some("NULL"));
    }

    #[test]
    fn integer_forms_include_hex_and_decimal_leading_zero() {
        let doc = parse_one("a: 1\nb: 0x10\nc: 010\nd: +5");
        assert!(matches!(doc["a"], Yaml::Integer(1)));
        assert!(matches!(doc["b"], Yaml::Integer(16)));
        // Quirk: leading-zero is parsed as decimal 10, not octal and not a string.
        assert!(matches!(doc["c"], Yaml::Integer(10)));
        assert!(matches!(doc["d"], Yaml::Integer(5)));
    }

    #[test]
    fn tab_indentation_is_rejected() {
        // TASK-0112 delta: yaml-rust 0.4 accepted a tab-indented scalar as
        // a string containing the tab; yaml-rust2 rejects tabs used for
        // block indentation per YAML 1.2.
        let err = YamlLoader::load_from_str("a:\n\t- b").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("tabs disallowed"),
            "tab indentation error, got: {text}"
        );
    }

    #[test]
    fn multi_document_input_returns_each_document() {
        let docs = YamlLoader::load_from_str("a: 1\n---\nb: 2").unwrap();
        assert_eq!(docs.len(), 2);
        assert!(matches!(docs[0]["a"], Yaml::Integer(1)));
        assert!(matches!(docs[1]["b"], Yaml::Integer(2)));
    }

    #[test]
    fn malformed_flow_sequence_is_a_scan_error_with_position() {
        let err = YamlLoader::load_from_str("name: [unclosed").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("flow sequence") && text.contains("line 2"),
            "scan error names the construct and position, got: {text}"
        );
    }
}
