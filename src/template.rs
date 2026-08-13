//! Pure command template expansion.
//!
//! Expands `{{filepath}}`, `{{absolute_path}}`, `{{relative_filepath}}` and
//! `{{relative_path}}` placeholders inside commands. This module has no YAML
//! parsing and no console output: unknown variables are collected and
//! reported to the caller, which decides how to present them.

pub struct TemplateOptions {
    pub filepath: Option<String>,
    pub current_dir: String,
}

pub struct TemplateOutput {
    pub commands: Vec<String>,
    pub unknown_variables: Vec<String>,
}

pub fn template(commands: Vec<String>, opts: TemplateOptions) -> TemplateOutput {
    let filepath = match opts.filepath {
        Some(val) => val,
        None => "".to_owned(),
    };

    let mut unknown_variables = vec![];

    let expanded = commands
        .iter()
        .map(|c| {
            if c.contains("{{") {
                c.split("{{")
                    .map(|part| {
                        if part.contains("}}") {
                            let parts: Vec<&str> = part.split("}}").collect();
                            let tpl = parts[0].trim();
                            let rest = parts[1];

                            match tpl {
                                "filepath" | "absolute_path" => format!("{}{}", &filepath, rest),
                                "relative_filepath" | "relative_path" => {
                                    let relative_path =
                                        &filepath.replace(&format!("{}/", &opts.current_dir), "");
                                    format!("{}{}", relative_path, rest)
                                }
                                _ => {
                                    unknown_variables.push(tpl.to_owned());
                                    format!("{}{}{}{}", "{{", parts[0], "}}", parts[1])
                                }
                            }
                        } else {
                            part.to_owned()
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("")
            } else {
                c.to_owned()
            }
        })
        .collect();

    TemplateOutput {
        commands: expanded,
        unknown_variables,
    }
}

#[cfg(test)]
mod tests {
    use super::{template, TemplateOptions};

    #[test]
    fn it_replaces_filepath_tpl_with_absolute_filepath() {
        let output = template(
            vec![
                "cargo tests {{filepath}}".to_owned(),
                "echo {{filepath}}".to_owned(),
                "make tests {{filepath}}".to_owned(),
            ],
            TemplateOptions {
                filepath: Some("tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.commands,
            vec![
                "cargo tests tests/foo.rs",
                "echo tests/foo.rs",
                "make tests tests/foo.rs"
            ]
        );
        assert!(output.unknown_variables.is_empty());

        let output = template(
            vec![
                "cargo tests {{filepath}}".to_owned(),
                "echo {{filepath}}".to_owned(),
                "make tests {{filepath}}".to_owned(),
            ],
            TemplateOptions {
                filepath: Some("/bar/baz/tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.commands,
            vec![
                "cargo tests /bar/baz/tests/foo.rs",
                "echo /bar/baz/tests/foo.rs",
                "make tests /bar/baz/tests/foo.rs"
            ]
        );
    }

    #[test]
    fn it_replaces_relative_filepath_tpl_with_relative_filepath() {
        let output = template(
            vec![
                "cargo tests {{relative_filepath}}".to_owned(),
                "git add {{relative_path}}".to_owned(),
                "echo {{filepath}}".to_owned(),
                "make tests {{absolute_path}}".to_owned(),
            ],
            TemplateOptions {
                filepath: Some("/foo/bar/tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.commands,
            vec![
                "cargo tests tests/foo.rs",
                "git add tests/foo.rs",
                "echo /foo/bar/tests/foo.rs",
                "make tests /foo/bar/tests/foo.rs"
            ]
        );
        assert!(output.unknown_variables.is_empty());
    }

    #[test]
    fn it_reports_unknown_template_variables() {
        let output = template(
            vec![
                "echo {{filepath}}".to_owned(),
                "echo {{mystery}}".to_owned(),
                "echo {{mystery}} again".to_owned(),
            ],
            TemplateOptions {
                filepath: Some("tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.commands,
            vec![
                "echo tests/foo.rs",
                "echo {{mystery}}",
                "echo {{mystery}} again"
            ]
        );
        // One report per occurrence, mirroring the previous warn-per-variable behavior.
        assert_eq!(output.unknown_variables, vec!["mystery", "mystery"]);
    }
}
