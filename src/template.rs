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
        .map(|c| expand_command(c, &filepath, &opts.current_dir, &mut unknown_variables))
        .collect();

    TemplateOutput {
        commands: expanded,
        unknown_variables,
    }
}

/// Expands command templates inside an execution command line, preserving the
/// argv boundary for ad-hoc `exec` rules: each argv element is expanded
/// independently, and the result stays an argv vector (never joined).
pub fn template_line(
    command: crate::rules::CommandLine,
    opts: TemplateOptions,
) -> TemplateLineOutput {
    let filepath = match opts.filepath {
        Some(val) => val,
        None => "".to_owned(),
    };

    let mut unknown_variables = vec![];

    let expanded = match command {
        crate::rules::CommandLine::Shell(command) => {
            crate::rules::CommandLine::Shell(expand_command(
                &command,
                &filepath,
                &opts.current_dir,
                &mut unknown_variables,
            ))
        }
        crate::rules::CommandLine::Argv(argv) => crate::rules::CommandLine::Argv(
            argv.into_iter()
                .map(|arg| {
                    expand_command(&arg, &filepath, &opts.current_dir, &mut unknown_variables)
                })
                .collect(),
        ),
    };

    TemplateLineOutput {
        command: expanded,
        unknown_variables,
    }
}

pub struct TemplateLineOutput {
    pub command: crate::rules::CommandLine,
    pub unknown_variables: Vec<String>,
}

fn expand_command(
    command: &str,
    filepath: &str,
    current_dir: &str,
    unknown_variables: &mut Vec<String>,
) -> String {
    if command.contains("{{") {
        command
            .split("{{")
            .map(|part| {
                if part.contains("}}") {
                    let parts: Vec<&str> = part.split("}}").collect();
                    let tpl = parts[0].trim();
                    let rest = parts[1];

                    match tpl {
                        "filepath" | "absolute_path" => format!("{}{}", &filepath, rest),
                        "relative_filepath" | "relative_path" => {
                            let relative_path = &filepath.replace(&format!("{}/", current_dir), "");
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
        command.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{template, TemplateOptions};

    #[test]
    fn it_expands_filepath_template_inside_each_argv_element() {
        use crate::rules::CommandLine;

        let output = super::template_line(
            CommandLine::Argv(vec![
                "echo".to_owned(),
                "changed: {{filepath}}".to_owned(),
                "--path={{relative_filepath}}".to_owned(),
            ]),
            TemplateOptions {
                filepath: Some("/foo/bar/tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.command,
            CommandLine::Argv(vec![
                "echo".to_owned(),
                "changed: /foo/bar/tests/foo.rs".to_owned(),
                "--path=tests/foo.rs".to_owned(),
            ])
        );
        assert!(output.unknown_variables.is_empty());
    }

    #[test]
    fn it_expands_filepath_template_inside_shell_command_line() {
        use crate::rules::CommandLine;

        let output = super::template_line(
            CommandLine::Shell("echo {{filepath}}".to_owned()),
            TemplateOptions {
                filepath: Some("tests/foo.rs".to_owned()),
                current_dir: "/foo/bar".to_owned(),
            },
        );

        assert_eq!(
            output.command,
            CommandLine::Shell("echo tests/foo.rs".to_owned())
        );
    }

    #[test]
    fn it_reports_unknown_variables_in_argv_elements() {
        use crate::rules::CommandLine;

        let output = super::template_line(
            CommandLine::Argv(vec!["echo {{unknown_var}}".to_owned()]),
            TemplateOptions {
                filepath: Some("x".to_owned()),
                current_dir: "/".to_owned(),
            },
        );

        assert_eq!(output.unknown_variables, vec!["unknown_var".to_owned()]);
    }

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
