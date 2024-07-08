use std::io::prelude::*;

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
                        && output.contains("All tasks finished successfully")
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
All tasks finished successfully.
Funzzy: finished in 0.0s
[2J
Funzzy: echo 'this file has changed: $PWD/examples/workdir/trigger-watcher.txt' 

this file has changed: $PWD/examples/workdir/trigger-watcher.txt

Funzzy: cat '$PWD/examples/workdir/trigger-watcher.txt' || echo 'nothing to run' 

test_content

Funzzy: echo '$PWD/examples/workdir/trigger-watcher.txt' | sed -r s/trigger/foobar/ 

$PWD/examples/workdir/foobar-watcher.txt
Funzzy results ----------------------------
All tasks finished successfully.
Funzzy: finished in 0.0s";

            assert_eq!(
                setup::clean_output(&output),
                expected.replace("$PWD", &dir.to_string_lossy())
            )
        },
    );
}
