use std::process::{ Command, Stdio };

fn command_parser(command: String) -> Vec<Command> {
    let formatted = command.replace("'", " ' ").replace("\"", " \" ");
    let mut tokens: Vec<&str> = formatted.split(' ').collect();

    let init = tokens.remove(0);
    let mut current_cmd = Command::new(init);
    while tokens.len() > 0 {
        let token = tokens.remove(0);
        match token.clone() {
            "|" | "||" | "&" | "&&" => {
                current_cmd.stdout(Stdio::piped());

                let cmdname = tokens.remove(0);
                let mut cmd = Command::new(cmdname);
                let result = current_cmd.spawn().unwrap().stdout.unwrap();
                cmd.stdin(Stdio::from(result));
                current_cmd = cmd;
            }

            "\"" => {
                let mut string_arg = String::new();
                while tokens.len() > 0 {
                    let token = tokens.remove(0);
                    if token == "\"" {
                        string_arg.pop();
                        current_cmd.arg(string_arg);
                        break;
                    }
                    string_arg.push_str(token);
                    string_arg.push_str(" ");
                }
            }

            "'" => {
                let mut string_arg = String::new();
                while tokens.len() > 0 {
                    let token = tokens.remove(0);
                    if token == "'" {
                        string_arg.pop();
                        current_cmd.arg(string_arg);
                        break;
                    }
                    string_arg.push_str(token);
                    string_arg.push_str(" ");
                }
            }

            arg if arg.len() > 0 => {
                current_cmd.arg(arg);
            }

            _ => {}
        }
    }

    vec![current_cmd]
}

pub fn execute(command_line: String) -> Result<(), String> {
    println!(" ----- funzzy running: {} -------", command_line);
    let commands = command_parser(command_line);

    for mut cmd in commands {
        if let Err(error) = cmd.status() {
            return Err(String::from(error.to_string()));
        }
    }

    return Ok(());
}

#[test]
fn it_executes_a_command() {
    let result = match execute(String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(msg) => panic!(msg),
    };

    assert!(result)
}

#[test]
fn it_creates_commands() {
    let result = command_parser(String::from("cargo build"));

    let mut expected = Command::new("cargo");
    expected.arg("build");
    assert_eq!(format!("{:?}", expected), format!("{:?}", result[0]))
}

#[test]
fn it_creates_commands_with_more_than_one_arg() {
    let result = command_parser(String::from("cargo build --verbose"));

    let mut expected = Command::new("cargo");
    expected.arg("build");
    expected.arg("--verbose");
    assert_eq!(format!("{:?}", expected), format!("{:?}", result[0]))
}

#[test]
fn it_accept_nested_commands_and_return_the_latest() {
    let result = command_parser(String::from("cargo build --verbose && cargo test"));

    let mut cmd2 = Command::new("cargo");
    cmd2.arg("test");

    let commands = vec![cmd2];

    assert_eq!(format!("{:?}", commands), format!("{:?}", result))
}


#[test]
fn it_allows_piping_outputs() {
    let mut commands = command_parser(
        String::from("echo 'foo' | sed 's/foo/bar/g'")
    );

    let mut child = commands.remove(0);

    let output = child.stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start sed process")
        .wait_with_output()
        .expect("Failed to wait on sed");

    let result = output.stdout.as_slice();

    assert_eq!("bar\n", String::from_utf8_lossy(result));
}

#[test]
fn it_accept_strings_as_arguments() {
    let result = command_parser(String::from("echo 'foo bar baz'"));

    let mut cmd2 = Command::new("echo");
    cmd2.arg("foo bar baz");

    let commands = vec![cmd2];

    assert_eq!(format!("{:?}", commands), format!("{:?}", result))
}
