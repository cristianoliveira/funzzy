use std::io::prelude::*;

use pretty_assertions::assert_eq;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_replaces_filepath_template_with_changed_file() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_replaces_filepath_template_with_changed_file.log",
            example_file: "examples/tasks-with-filepath-template.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("-t")
                .arg("@absolute")
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Running on init commands.")
                        && output.contains("Funzzy results")
                },
                "Funzzy has not been started with verbose mode {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("this file has changed")
                        && output.contains("foobar-watcher.txt")
                        && output.contains("Success; Completed")
                },
                "was not possible to find filepath: {}",
                output
            );

            let dir = std::env::current_dir().expect("failed to get current dir");
            let expected = "Funzzy: Running on init commands.

Funzzy: echo 'this file has changed: ' 

this file has changed: 

Funzzy: cat '' || echo 'nothing to run' 

nothing to run

Funzzy: echo '' | sed -r s/trigger/foobar/ 


Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Duration: 0.0000s
[2J
Funzzy: echo 'this file has changed: $PWD/examples/workdir/trigger-watcher.txt' 

this file has changed: $PWD/examples/workdir/trigger-watcher.txt

Funzzy: cat '$PWD/examples/workdir/trigger-watcher.txt' || echo 'nothing to run' 

test_content

Funzzy: echo '$PWD/examples/workdir/trigger-watcher.txt' | sed -r s/trigger/foobar/ 

$PWD/examples/workdir/foobar-watcher.txt
Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Duration: 0.0000s";

            assert_eq!(
                setup::clean_output(&output),
                expected.replace("$PWD", &dir.to_string_lossy())
            )
        },
    );
}

#[test]
fn it_replaces_relative_path_relative_to_the_cunrrent_dir() {
    setup::with_example(
        setup::Options {
            output_file: "it_replaces_relative_path_relative_to_the_cunrrent_dir.log",
            example_file: "examples/tasks-with-filepath-template.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("-t")
                .arg("@relative")
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Running on init commands.")
                        && output.contains("Funzzy results")
                },
                "Funzzy has not been started with verbose mode {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("echo 'examples/workdir/trigger-watcher.txt'")
                        && output.match_indices("Funzzy results").count() == 2
                },
                "output: {}\nreason: was not possible to echo with relative path",
                output
            );

            let dir = std::env::current_dir().expect("failed to get current dir");
            let expected = "Funzzy: Running on init commands.
Funzzy warning: Unknown template variable 'foobar'.

Funzzy: echo '' 



Funzzy: echo '' 



Funzzy: echo 'this is also valid:  (nice!)' 

this is also valid:  (nice!)

Funzzy: echo 'this is invalid: {{ foobar }} (no!)' 

this is invalid: {{ foobar }} (no!)
Funzzy results ----------------------------
Success; Completed: 4; Failed: 0; Duration: 0.0000s
[2JFunzzy warning: Unknown template variable 'foobar'.

Funzzy: echo '$PWD/examples/workdir/trigger-watcher.txt' 

$PWD/examples/workdir/trigger-watcher.txt

Funzzy: echo 'examples/workdir/trigger-watcher.txt' 

examples/workdir/trigger-watcher.txt

Funzzy: echo 'this is also valid: $PWD/examples/workdir/trigger-watcher.txt (nice!)' 

this is also valid: $PWD/examples/workdir/trigger-watcher.txt (nice!)

Funzzy: echo 'this is invalid: {{ foobar }} (no!)' 

this is invalid: {{ foobar }} (no!)
Funzzy results ----------------------------
Success; Completed: 4; Failed: 0; Duration: 0.0000s";

            assert_eq!(
                setup::clean_output(&output),
                expected.replace("$PWD", &dir.to_string_lossy())
            )
        },
    );
}
