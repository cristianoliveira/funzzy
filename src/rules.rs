extern crate glob;
extern crate yaml_rust;

use cli;
use yaml;

use self::glob::Pattern;
use self::yaml_rust::Yaml;
use self::yaml_rust::YamlLoader;
use std::fs::File;
#[warn(unused_imports)]
use std::io::prelude::*;

#[derive(Debug, Clone)]
pub struct Rules {
    pub name: String,

    commands: Vec<String>,
    watch_patterns: Vec<String>,
    ignore_patterns: Vec<String>,
    run_on_init: bool,
}

impl Rules {
    pub fn new(
        name: String,
        commands: Vec<String>,
        watches: Vec<String>,
        ignores: Vec<String>,
        run_on_init: bool,
    ) -> Self {
        Rules {
            name: name,
            commands: commands,
            watch_patterns: watches,
            ignore_patterns: ignores,
            run_on_init: run_on_init,
        }
    }

    pub fn from(yaml: &Yaml) -> Self {
        yaml::validate(yaml, "run");
        yaml::validate(yaml, "change");

        Rules {
            name: yaml::extract_strings(&yaml["name"])[0].clone(),
            commands: yaml::extract_strings(&yaml["run"]),
            watch_patterns: yaml::extract_strings(&yaml["change"]),
            ignore_patterns: yaml::extract_strings(&yaml["ignore"]),
            run_on_init: yaml::extract_bool(&yaml["run_on_init"]),
        }
    }

    pub fn watch(&self, path: &str) -> bool {
        self.watch_patterns
            .iter()
            .any(|watch| pattern(watch).matches(path))
    }

    pub fn ignore(&self, path: &str) -> bool {
        self.ignore_patterns
            .iter()
            .any(|ignore| pattern(ignore).matches(path))
            || false
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }

    pub fn run_on_init(&self) -> bool {
        self.run_on_init
    }
}

pub fn from_yaml(file_content: &str) -> Result<Vec<Rules>, String> {
    let items = YamlLoader::load_from_str(file_content).unwrap();
    match items[0] {
        Yaml::Array(ref items) => Ok(items.iter().map(|item| Rules::from(item)).collect()),
        _ => Err("You must have at last one item in the yaml.".to_owned()),
    }
}

pub fn from_string(patterns: String, command: &String) -> Vec<Rules> {
    // get current directory
    let current_dir = std::env::current_dir().expect("Cannot get current directory");
    let current_dir_str = current_dir
        .to_str()
        .expect("Cannot convert current directory to string");

    let watches = patterns
        .lines()
        .filter(|line| line.len() > 1)
        .map(|line| {
            let path = std::path::Path::new(&line);

            let full_path = if path.starts_with(".") {
                std::path::Path::new(&current_dir_str).join(&line[2..])
            } else {
                std::path::Path::new(&current_dir_str).join(&line)
            };

            if full_path.is_dir() {
                return full_path
                    .join("**")
                    .to_str()
                    .expect(format!("Cannot convert {:?} to path with wildcard", line).as_str())
                    .to_owned();
            }

            full_path
                .to_str()
                .expect(format!("Cannot convert {:?} to absolute path", line).as_str())
                .to_owned()
        })
        .collect();

    vec![Rules::new(
        "unnamed".to_owned(),
        vec![command.clone()],
        watches,
        vec![],
        false,
    )]
}

pub fn from_file(filename: &str) -> Result<Vec<Rules>, String> {
    match File::open(filename) {
        Ok(mut file) => {
            let mut content = String::new();
            file.read_to_string(&mut content)
                .expect(format!("Cannot read file {}", filename).as_str());

            match from_yaml(&content) {
                Err(err) => Err(err),
                rules => rules,
            }
        }

        Err(err) => Err(format!(
            "File {} cannot be opened. Cause: {}",
            cli::watch::DEFAULT_FILENAME,
            err
        )
        .to_owned()),
    }
}

fn pattern(pattern: &str) -> Pattern {
    Pattern::new(&format!("**/{}", pattern)).expect("Pattern error.")
}

#[cfg(test)]
mod tests {
    extern crate yaml_rust;

    use self::yaml_rust::YamlLoader;
    use super::from_string;
    use super::Rules;

    #[test]
    fn it_is_watching_path_tests() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert_eq!(true, rule.watch("tests/foo.rs"));
    }

    #[test]
    fn it_is_not_watching_path_test() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert_eq!(false, rule.watch("tests/foo.rs"));
    }

    #[test]
    fn it_accepts_run_on_init() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
          run_on_init: true
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert!(rule.run_on_init());
    }

    #[test]
    fn it_accepts_false_for_run_on_init() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
          run_on_init: false
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_defaults_run_on_init_to_false() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_is_ignoring_path_tests() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'bla/**'
          ignore: 'tests/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert_eq!(true, rule.ignore("tests/foo.rs"));
    }

    #[test]
    fn it_is_not_ignoring_path_test() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'bla/**'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert_eq!(false, rule.ignore("tests/foo.rs"));
    }

    #[test]
    fn it_loads_from_args() {
        let file_content = "
        - name: my test
          run: 'cargo tests'
          change: 'bla/**'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        let result = rule.commands();
        assert_eq!(vec!["cargo tests"], result);
    }

    #[test]
    #[should_panic]
    fn it_validates_the_run_key() {
        let file_content = "
        - name: my source
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let content = YamlLoader::load_from_str(file_content).unwrap();
        Rules::from(&content[0][0]);
    }

    #[test]
    #[should_panic]
    fn it_validates_the_when_change_key() {
        let file_content = "
        - name: my source
          run: make test
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let content = YamlLoader::load_from_str(file_content).unwrap();
        Rules::from(&content[0][0]);
    }

    #[test]
    fn it_filters_empty_or_one_character_path() {
        let content = "./foo\n./bar\n.\n./baz\n";
        let rules = from_string(String::from(content), &String::from("cargo test"));
        assert!(rules[0].watch("foo"));
        assert!(rules[0].watch("bar"));
        assert!(rules[0].watch("baz"));
        assert!(!rules[0].watch("."));
    }
}
