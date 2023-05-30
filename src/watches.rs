use rules::Rules;

/// # Watches
///
/// Represents all rules in the yaml config loaded.
///
#[derive(Debug)]
pub struct Watches {
    rules: Vec<Rules>,
}
impl Watches {
    pub fn new(rules: Vec<Rules>) -> Self {
        Watches { rules }
    }

    /// Returns the commands for first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<Vec<String>>> {
        let cmds = self
            .rules
            .iter()
            .filter(|r| !r.ignore(path) && r.watch(path))
            .map(|r| r.commands())
            .collect::<Vec<Vec<String>>>();

        match cmds.len() {
            0 => None,
            _ => Some(cmds),
        }
    }

    /// Returns the commands for the rules that should run on init
    ///
    pub fn run_on_init(&self) -> Option<Vec<Vec<String>>> {
        let cmds = self
            .rules
            .iter()
            .filter(|r| r.run_on_init())
            .map(|r| r.commands())
            .collect::<Vec<Vec<String>>>();

        match cmds.len() {
            0 => None,
            _ => Some(cmds),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate glob;
    extern crate notify;
    extern crate yaml_rust;

    use super::*;
    use rules;

    #[test]
    fn it_loads_from_args() {
        let args = String::from("cargo build");
        let watches = Watches::new(rules::from_string("**".to_owned(), &args));

        assert!(watches.watch("src/main.rs").is_some());
        assert!(watches.watch("test/main.rs").is_some());
        assert!(watches.watch(".").is_some());

        let result = watches.watch(".").unwrap();
        assert_eq!(vec!["cargo build"], result[0]);
    }

    #[test]
    fn it_watches_test_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches
            .watch("/Users/crosa/others/funzzy/tests/test.rs")
            .is_some());
        assert!(watches.watch("tests/tests.rs").is_some());
        assert!(watches.watch("tests/ruby.rb").is_some());
        assert!(watches.watch("tests/folder/other.rs").is_some())
    }

    #[test]
    fn it_watches_specific_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: './tests/foo/bar.rs'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("./tests/foo/bar.rs").is_some())
    }

    #[test]
    fn it_doesnot_watch_test_path() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));

        assert!(watches
            .watch("/Users/crosa/others/funzzy/events.yaml")
            .is_none());
        assert!(watches.watch("tests/").is_none());
        assert!(watches.watch("tests/test.rs").is_none());
        assert!(watches.watch("tests/folder/other.rs").is_none());
    }

    #[test]
    fn it_creates_a_list_of_shell_commands() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        let result = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["cargo build"], result[0])
    }

    #[test]
    fn it_works_with_multiples_items() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));

        let result = watches.watch("test/test.rs").unwrap();
        assert_eq!(vec!["cargo test"], result[0]);

        let result_src = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["cargo build"], result_src[0]);
    }

    #[test]
    fn it_allows_many_rules_watching_same_path() {
        let file_content = "
        - name: same path
          run: 'echo same'
          change: '**'

        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));

        let result = watches.watch("test/test.rs").unwrap();
        assert_eq!(vec!["echo same"], result[0]);
        assert_eq!(vec!["cargo test"], result[1]);

        let result_multiple = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["echo same"], result_multiple[0]);
        assert_eq!(vec!["cargo build"], result_multiple[1]);
    }

    #[test]
    fn it_ignores_pattern() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: 'src/test/**'
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_a_list_of_patterns() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/tmp/test.txt").is_none());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_returns_on_init_rules() {
        let file_content = "
            - name: my source
              run: 'cargo build'
              change: 'src/**'
              run_on_init: true

            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'
              run_on_init: true

            - name: other
              run: 'cargo test'
              change: 'test/**'
            ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        let results = watches.run_on_init().unwrap();

        assert_eq!(results[0], vec!["cargo build".to_string(),]);
        assert_eq!(
            results[1],
            vec!["cat foo".to_string(), "cat bar".to_string(),]
        );
    }
}
