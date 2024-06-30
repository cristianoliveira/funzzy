use std::io::prelude::*; 

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_executes_tasks_on_init_when_configured() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_executes_tasks_on_init_when_configured.log",
            example_file: "examples/list-of-tasks-run-on-init.yml",
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

                    output.contains("Funzzy results")
                },
                "No task in the example was configured with run_on_init {}",
                output
            );


            let clear_noise = "[H[J";
            assert_eq!(
                output.replace(clear_noise, ""),
                "Funzzy: Running on init commands.

Funzzy: echo 'running on init first' 

running on init first

Funzzy: echo \"run on init sencod\" 

run on init sencod
Funzzy results ----------------------------
All tasks finished successfully.
Funzzy: Watching...
"
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("should not run on init but on change")
                },
                "OUTPUT: {}",
                output
            );

        },
    );
}
