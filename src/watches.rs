use crate::rules::Rules;

/// # Watches
///
/// Represents all rules in the yaml config loaded.
///
#[derive(Debug, Clone)]
pub struct Watches {
    rules: Vec<Rules>,
}
impl Watches {
    pub fn new(rules: Vec<Rules>) -> Self {
        Watches { rules }
    }

    /// Returns the commands for first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<Rules>> {
        let cmds = self
            .rules
            .iter()
            .cloned()
            .filter(|r| !r.ignore(path) && r.watch(path))
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Returns the commands for the rules that should run on init
    ///
    pub fn run_on_init(&self) -> Option<Vec<Rules>> {
        let cmds = self
            .rules
            .iter()
            .cloned()
            .filter(|r| r.run_on_init())
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Extract the directory to watch from a glob pattern.
    /// For example:
    /// - "src/**" -> "src"
    /// - "/tmp/**" -> "/tmp"
    /// - "examples/workdir/**/*" -> "examples/workdir"
    fn extract_watch_directory(pattern: &str, current_dir: &std::path::Path) -> String {
        let absolute_pattern = if pattern.starts_with("/") {
            pattern.to_string()
        } else {
            let mut abs = current_dir.to_path_buf();
            abs.push(pattern);
            abs.to_str().unwrap().to_string()
        };

        // Split by '/' and collect segments until we hit a glob metacharacter
        let mut segments = Vec::new();
        let is_absolute = absolute_pattern.starts_with('/');

        for segment in absolute_pattern.split('/') {
            if segment.contains('*')
                || segment.contains('?')
                || segment.contains('[')
                || segment.contains('{')
            {
                break;
            }
            if !segment.is_empty() {
                segments.push(segment);
            }
        }

        if segments.is_empty() {
            return current_dir.to_str().unwrap().to_string();
        }

        let mut result = String::new();
        if is_absolute {
            result.push('/');
        }
        result.push_str(&segments.join("/"));
        result
    }

    /// Returns the list of rules that contains absolute path
    ///
    pub fn paths_to_watch(&self) -> Option<Vec<String>> {
        let current_dir = std::env::current_dir().expect("Unable to get current directory");
        let mut paths = Vec::new();

        for rule in &self.rules {
            for pattern in rule.watch_patterns() {
                let dir = Self::extract_watch_directory(&pattern, &current_dir);
                if !paths.contains(&dir) {
                    paths.push(dir);
                }
            }
        }

        // Always watch current directory as fallback
        let current_dir_str = current_dir.to_str().unwrap().to_string();
        if !paths.contains(&current_dir_str) {
            paths.push(current_dir_str);
        }

        if !paths.is_empty() {
            Some(paths)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate glob;
    extern crate notify;
    extern crate yaml_rust;

    use super::*;
    use crate::rules;
    use std::env;

    fn get_absolute_path(path: &str) -> String {
        let mut absolute_path = env::current_dir().unwrap();
        absolute_path.push(path);
        absolute_path.to_str().unwrap().to_string()
    }

    #[test]
    fn it_loads_from_args() {
        let args = String::from("cargo build");
        let watches = Watches::new(
            rules::from_string(vec![".".to_owned()], args)
                .expect("Error while parsing rules from string"),
        );

        assert!(watches.watch(&get_absolute_path("src/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path("test/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path(".")).is_some());

        let result = rules::commands(watches.watch(&get_absolute_path(".")).unwrap());
        assert_eq!(vec!["cargo build"], result);
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
        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result[0])
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

        let result = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!("cargo test", result[0]);

        let result_src = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result_src[0]);
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

        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo build"], result);

        let result_multiple = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo test"], result_multiple);
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
        let results = rules::commands(watches.run_on_init().unwrap());

        assert_eq!(
            results,
            vec![
                "cargo build".to_string(),
                "cat foo".to_string(),
                "cat bar".to_string(),
            ]
        );
    }

    #[test]
    fn it_returns_rules_with_absolute_path_and_current_dir() {
        let file_content = "
            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'

            - name: rule with absolute path
              run: 'cargo build'
              change: 
                - 'src/**'
                - '/tmp/**'
                - '/User/**'

            - name: it does not consider the ignored rules
              run: 'cargo test'
              change: 'test/**'
              ignored: '/test/**'

            - name: another with absolute path
              run: echo 'absolute paths'
              change: 
                - '/dev/**'
                - '/usr/**'
                - '/etc/**'
            ";
        let watches = Watches::new(rules::from_yaml(&file_content).expect("Error parsing yaml"));
        let results = watches.paths_to_watch().expect("No rules found");

        let current_dir = std::env::current_dir().expect("Unable to get current directory");
        // Compute expected directories: all patterns converted to directories, plus current_dir
        let mut expected = Vec::new();
        let patterns = vec![
            "src/**", "src/**", "/tmp/**", "/User/**", "test/**", "/dev/**", "/usr/**", "/etc/**",
        ];
        for pattern in patterns {
            let dir = Watches::extract_watch_directory(pattern, &current_dir);
            if !expected.contains(&dir) {
                expected.push(dir);
            }
        }
        // Add current directory if not already present (it should be added by paths_to_watch)
        let current_dir_str = current_dir.to_str().unwrap().to_string();
        if !expected.contains(&current_dir_str) {
            expected.push(current_dir_str);
        }

        assert_eq!(results.len(), expected.len());
        // Order should match iteration order
        for (i, expected_dir) in expected.iter().enumerate() {
            assert_eq!(&results[i], expected_dir);
        }
    }

    #[test]
    fn it_returns_an_error_when_fail_to_load_config_file() {
        // assert!(rules::from_yaml(
        //     &r#"
        // - name: run tests
        //   run: [
        //     "yarn test {{filepath}}",
        //     "echo '{{filepath}}' | sed -r 's\/.tsx/\/'"
        //   ]
        //   change: 'src/**'
        // "#
        // )
        // .is_err());

        assert!(rules::from_yaml(
            &r#"
        - name: run tests
          run: [
            "yarn test {{filepath}}", 
          change: 'src/**'
        "#
        )
        .is_err());

        assert!(rules::from_yaml(
            &r#"
        - name: other
          run: 'cargo test'
          change: 'test/**'
        "#
        )
        .is_ok());
    }
}
