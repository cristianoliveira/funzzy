extern crate yaml_rust;
extern crate glob;

use yaml;

use self::yaml_rust::Yaml;
use self::yaml_rust::YamlLoader;
use self::glob::Pattern;

#[derive(Debug)]
pub struct Rules {
    commands: Vec<String>,
    watch_patterns: Vec<String>,
    ignore_patterns: Vec<String>,
}
impl Rules {
    pub fn new(commands: Vec<String>, watches: Vec<String>, ignores: Vec<String>) -> Self {
        Rules {
            commands: commands,
            watch_patterns: watches,
            ignore_patterns: ignores
        }
    }
    pub fn from(yaml: &Yaml) -> Self {
        yaml::validate(yaml, "run");
        yaml::validate(yaml, "change");

        Rules {
            commands: yaml::extract_strings(&yaml["run"]),
            watch_patterns: yaml::extract_strings(&yaml["change"]),
            ignore_patterns: yaml::extract_strings(&yaml["ignore"])
        }
    }

    pub fn watch(&self, path: &str) -> bool {
        self.watch_patterns.iter()
            .any(|watch| pattern(watch).matches(path))
    }

    pub fn ignore(&self, path: &str) -> bool {
        self.ignore_patterns.iter()
            .any(|ignore| pattern(ignore).matches(path)) || false
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }
}

pub fn from_yaml(file_content: &str) -> Vec<Rules> {
    let items = YamlLoader::load_from_str(file_content).unwrap();
    match items[0] {
        Yaml::Array(ref items) => items.iter()
                                       .map(|rule| Rules::from(rule))
                                       .collect(),
        _ => panic!("You must have at last one item in the yaml.")
    }
}

pub fn from_string(patterns: String, command: &String) -> Vec<Rules> {
    let watches = patterns.lines()
                        .filter(|line| line.len() > 1)
                        .map(|line| format!("**/{}", &line[2..]))
                        .collect();
    vec![Rules::new(vec![command.clone()], watches, vec![])]
}

fn pattern(pattern: &str) -> Pattern {
    Pattern::new(&format!("**/{}", pattern)).expect("Pattern error.")
}

#[cfg(test)]
mod tests {
    extern crate yaml_rust;

    use super::Rules;
    use super::from_string;
    use self::yaml_rust::YamlLoader;

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
