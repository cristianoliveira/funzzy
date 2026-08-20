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
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd
                .args(["watch", "@absolute"])
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
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

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

            let dir = fixture;
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
Success; Completed: 3; Failed: 0; Duration: 0.0000s"
                .replace("$PWD", &dir.to_string_lossy());

            // Waiting for individual markers races the final summary flush;
            // wait until the whole log settles to the expected content.
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    setup::strip_ansi_codes(&setup::clean_output(&output)) == expected
                },
                "was not possible to find filepath: {}",
                output
            );

            assert_eq!(
                setup::strip_ansi_codes(&setup::clean_output(&output)),
                expected
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
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd
                .args(["watch", "@relative"])
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
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

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

            let dir = fixture;
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
Success; Completed: 4; Failed: 0; Duration: 0.0000s"
                .replace("$PWD", &dir.to_string_lossy());

            // Same as the absolute-path test: wait for the whole log to
            // settle to the expected content, not just marker lines.
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    setup::strip_ansi_codes(&setup::clean_output(&output)) == expected
                },
                "output: {}\nreason: was not possible to echo with relative path",
                output
            );

            assert_eq!(
                setup::strip_ansi_codes(&setup::clean_output(&output)),
                expected
            )
        },
    );
}
