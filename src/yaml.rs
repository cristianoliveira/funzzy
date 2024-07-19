extern crate yaml_rust;

use self::yaml_rust::Yaml;

pub fn extract_strings(yaml: &Yaml) -> Vec<String> {
    match yaml.clone() {
        Yaml::Array(ref items) => items
            .iter()
            .map(|i| String::from(i.as_str().unwrap_or_else(|| "_invalid_value_")))
            .collect(),
        Yaml::String(ref item) => vec![String::from(item.as_str())],
        Yaml::BadValue => vec![],
        unknown => {
            println!("Error: config file has an unknown type. {:?}", unknown);
            panic!("Interrupted due the previou error.")
        }
    }
}

/// It tries to find a boolean otherwise if defaults to false
///
pub fn extract_bool(yaml: &Yaml) -> bool {
    match yaml.clone() {
        Yaml::Boolean(item) => item,
        _ => false,
    }
}
