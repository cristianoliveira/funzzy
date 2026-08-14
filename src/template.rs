//! Pure command template expansion.
//!
//! Expands `{{filepath}}`, `{{absolute_path}}`, `{{relative_filepath}}` and
//! `{{relative_path}}` placeholders inside commands. This module has no YAML
//! parsing and no console output: unknown variables are collected and
//! reported to the caller, which decides how to present them.

#[derive(Clone)]
pub struct TemplateOptions {
    pub filepath: Option<String>,
    /// Complete normalized changed-path set of the triggering batch
    /// (TASK-0031): exposed as `{{paths}}`. Empty for runs without a batch.
    pub paths: Vec<String>,
    pub current_dir: String,
}

/// Renders the complete changed-path set for `{{paths}}`: paths are
/// shell-escaped and space-joined so a single expansion stays one safe
/// argument list. Backward-compatible: `{{filepath}}` keeps the trigger path.
pub fn render_paths(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| shell_escape(path))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Single-quotes a path for shell use, escaping embedded single quotes.
fn shell_escape(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
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
    let paths = render_paths(&opts.paths);

    let mut unknown_variables = vec![];

    let expanded = commands
        .iter()
        .map(|c| {
            expand_command(
                c,
                &filepath,
                &paths,
                &opts.current_dir,
                &mut unknown_variables,
            )
        })
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
    let paths = render_paths(&opts.paths);

    let mut unknown_variables = vec![];

    let expanded = match command {
        crate::rules::CommandLine::Shell(command) => {
            crate::rules::CommandLine::Shell(expand_command(
                &command,
                &filepath,
                &paths,
                &opts.current_dir,
                &mut unknown_variables,
            ))
        }
        crate::rules::CommandLine::Argv(argv) => crate::rules::CommandLine::Argv(
            argv.into_iter()
                .map(|arg| {
                    expand_command(
                        &arg,
                        &filepath,
                        &paths,
                        &opts.current_dir,
                        &mut unknown_variables,
                    )
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
    paths: &str,
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
                        // TASK-0031: the complete normalized changed-path set of
                        // the triggering batch, shell-escaped and space-joined.
                        "paths" => format!("{}{}", paths, rest),
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
                paths: vec![],
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
                paths: vec![],
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
                paths: vec![],
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
                paths: vec![],
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
                paths: vec![],
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
                paths: vec![],
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
                paths: vec![],
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

#[cfg(test)]
mod paths_tests {
    use super::{render_paths, template, template_line, TemplateOptions};
    use crate::rules::CommandLine;

    #[test]
    fn render_paths_joins_and_shell_escapes_every_path() {
        assert_eq!(render_paths(&[]), "");
        assert_eq!(
            render_paths(&["a.txt".to_owned(), "b c.txt".to_owned()]),
            "'a.txt' 'b c.txt'"
        );
        // Embedded single quotes are escaped for shell safety.
        assert_eq!(render_paths(&["it's.rs".to_owned()]), "'it'\\''s.rs'");
    }

    #[test]
    fn paths_template_expands_to_the_complete_batch() {
        let output = template_line(
            CommandLine::Shell("echo {{paths}}".to_owned()),
            TemplateOptions {
                filepath: Some("a.txt".to_owned()),
                paths: vec!["a.txt".to_owned(), "b c.txt".to_owned()],
                current_dir: "/workspace".to_owned(),
            },
        );
        assert_eq!(
            output.command,
            CommandLine::Shell("echo 'a.txt' 'b c.txt'".to_owned())
        );
        assert!(output.unknown_variables.is_empty());
    }

    #[test]
    fn filepath_stays_backward_compatible_alongside_paths() {
        let output = template(
            vec!["echo {{filepath}}; echo {{paths}}".to_owned()],
            TemplateOptions {
                filepath: Some("trigger.txt".to_owned()),
                paths: vec!["trigger.txt".to_owned()],
                current_dir: "/workspace".to_owned(),
            },
        );
        assert_eq!(
            output.commands,
            vec!["echo trigger.txt; echo 'trigger.txt'".to_owned()]
        );
        assert!(output.unknown_variables.is_empty());
    }

    #[test]
    fn paths_renders_empty_when_no_batch() {
        let output = template(
            vec!["echo [{{paths}}]".to_owned()],
            TemplateOptions {
                filepath: None,
                paths: vec![],
                current_dir: "/workspace".to_owned(),
            },
        );
        assert_eq!(output.commands, vec!["echo []".to_owned()]);
    }
}
