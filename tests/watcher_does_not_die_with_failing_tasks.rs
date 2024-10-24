use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_watches_a_list_of_tasks_and_do_not_panic() {
    setup::with_example(
        setup::Options {
            example_file: "examples/list-of-failing-commands.yml",
            output_file: "test_it_watches_a_list_of_tasks_and_do_not_panic.log",
        },
        |fzz_cmd, mut output_file| {
            let mut output = String::new();
            let mut child = fzz_cmd.spawn().expect("failed to spawn sub process");
            defer!({
                child.kill().expect("failed to kill sub process");
            });

            wait_until!({
                output_file
                    .read_to_string(&mut output)
                    .expect("failed to read test output file");

                output.contains("Running on init commands.") && output.contains("Funzzy results")
            });

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_file
                        .read_to_string(&mut output)
                        .expect("failed to read test output file");

                    output.contains("Failed tasks: 4")
                },
                "failed failed less than 4 tasks {}",
                output
            );

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 


Funzzy: echo finally 

finally
Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Finished in 0.0s
[2J
Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo before 

before

Funzzy: exit 1 


Funzzy: cat foo/bar/baz 


Funzzy: exit 125 


Funzzy: echo after 

after

Funzzy: cat baz/bar/foo 


Funzzy: echo finally 

finally
Funzzy results ----------------------------
Failed tasks: 4
 - Command exit 1 has failed with exit status: 1
 - Command cat foo/bar/baz has failed with exit status: 1
 - Command exit 125 has failed with exit status: 125
 - Command cat baz/bar/foo has failed with exit status: 1
Finished in 0.0s",
                "failed to match {}",
                output
            );
        },
    );
}
