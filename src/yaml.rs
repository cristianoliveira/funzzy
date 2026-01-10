extern crate yaml_rust;

use crate::errors::{FzzError, Result};

use self::yaml_rust::Yaml;

pub fn extract_list(yaml: &Yaml, prop: &str) -> Result<Vec<String>> {
    match &yaml[prop] {
        Yaml::Array(ref items) => Ok(items
            .iter()
            .map(|i| String::from(i.as_str().unwrap_or_else(|| "_invalid_value_")))
            .collect()),
        Yaml::String(ref item) => Ok(vec![String::from(item.as_str())]),
        Yaml::BadValue => Err(FzzError::InvalidConfigError(
            format!(
                "Missing '{}' in rule\n```yaml\n{}\n```",
                prop,
                yaml_to_string(&yaml, 0),
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
                yaml_to_string(&yaml, 0),
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
                yaml_to_string(&yaml, 0),
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
                yaml_to_string(&yaml, 0),
            ),
            None,
            Some(
                "Check if the property is defined, with the right type and identation".to_string(),
            ),
        )),
    }
}

pub fn extract_optional_string(yaml: &Yaml, prop: &str) -> Result<Option<String>> {
    match &yaml[prop] {
        Yaml::BadValue => Ok(None),
        Yaml::String(ref item) => Ok(Some(String::from(item.as_str()))),
        unknown => Err(FzzError::InvalidConfigError(
            format!(
                "Invalid property '{}' in rule below
Expected 'String' but got: {:?}
```
{}
```",
                prop,
                get_type(unknown),
                yaml_to_string(&yaml, 0),
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
    use self::yaml_rust::YamlLoader;
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
