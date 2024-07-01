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
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Watching...")
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
            let expected = "Funzzy: Watching...

Funzzy: clear 


Funzzy: echo 'this file has changed: $PWD/examples/workdir/trigger-watcher.txt' 

this file has changed: $PWD/examples/workdir/trigger-watcher.txt

Funzzy: cat $PWD/examples/workdir/trigger-watcher.txt 

test_content

Funzzy: echo '$PWD/examples/workdir/trigger-watcher.txt' | sed -r s/trigger/foobar/ 

$PWD/examples/workdir/foobar-watcher.txt
Funzzy results ----------------------------
All tasks finished successfully.
";

            let clear_sign = "[H[J";
            assert_eq!(output.replace(clear_sign, ""), expected.replace("$PWD", &dir.to_string_lossy()));
        },
    );
}
