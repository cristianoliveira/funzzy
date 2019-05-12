extern crate yaml_rust;

use self::yaml_rust::Yaml;

pub fn extract_strings(yaml: &Yaml) -> Vec<String> {
    match yaml.clone() {
        Yaml::Array(ref items) => items
            .iter()
            .map(|i| String::from(i.as_str().unwrap()))
            .collect(),
        Yaml::String(ref item) => vec![String::from(item.as_str())],
        Yaml::BadValue => vec![],
        _ => panic!("Unkown format. Please review the yaml"),
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

pub fn validate(yaml: &Yaml, key: &str) {
    if yaml[key].is_badvalue() {
        println!("File has a bad format. (Key {} not found)", key);
        panic!("Panicate due the previou error.")
    }
}
