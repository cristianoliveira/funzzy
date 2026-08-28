use std::collections::HashMap;
use std::io::prelude::*;

// use pretty_assertions::assert_eq;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_executes_tasks_on_init_when_configured() {
    setup::with_env(
        HashMap::from([("FUNZZY_COLORED".to_string(), "1".to_string())]),
        || {
            setup::with_example(
                setup::Options {
                    output_file: "test_it_executes_tasks_on_init_when_configured.log",
                    example_file: "examples/jobs-run-on-init.yml",
                },
                |fzz_cmd, mut output_log, fixture| {
                    let mut child = fzz_cmd
                        .env("_TEST_FUNZZY_COLORED", "1")
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

                            output.contains("Funzzy results")
                        },
                        "No task in the example was configured with run_on_init {}",
                        output
                    );

                    // Concurrent test binaries write the same
                    // `examples/workdir/trigger-watcher.txt`, so this child can
                    // fire extra runs beyond the init one we waited for. Assert
                    // the shape of the FIRST run only: init ran, and the tasks
                    // fired. Colors are part of the contract here (the child is
                    // explicitly told to colorize).
                    let first_run = &output[..output.find("Funzzy results").unwrap()];
                    let first_run = setup::clean_output(first_run);
                    assert!(
                        first_run.contains("Running on init commands"),
                        "must run tasks on init: {}",
                        first_run
                    );
                    assert!(first_run.contains("\u{1b}[34mFunzzy\u{1b}[0m"));
                    assert!(first_run.contains("running on init first"));
                    assert!(first_run.contains("run on init sencod"));
                    assert!(first_run.contains("only run on init"));
                    assert!(
                        !first_run.contains("should not run on init but on change"),
                        "non-init task must not run on init: {}",
                        first_run
                    );

                    // FIXME: this should not be needed sleep 5s
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

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

            Ok(())
        },
    )
    .expect("failed to run test");
}

#[test]
fn test_it_does_not_executes_tasks_on_init_when_no_run_on_init_flag() {
    setup::with_env(
        HashMap::from([("FUNZZY_COLORED".to_string(), "1".to_string())]),
        || {
            setup::with_example(
                setup::Options {
                    output_file:
                        "test_it_does_not_executes_tasks_on_init_when_no_run_on_init_flag.log",
                    example_file: "examples/jobs-run-on-init.yml",
                },
                |fzz_cmd, mut output_log, fixture| {
                    let mut child = fzz_cmd
                        .arg("--no-run-on-init")
                        .env("_TEST_FUNZZY_COLORED", "1")
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

                            !output.contains("Running on init commands")
                                && output.contains("Watching...")
                        },
                        "No task in the example was configured with run_on_init {}",
                        output
                    );

                    // FIXME: this should not be needed sleep 5s
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

                    // Wait for the whole first run to finish (the "Funzzy
                    // results" summary marker) before slicing: the change task's
                    // own output can appear mid-run, and slicing on the summary
                    // before it is written panics under load.
                    wait_until!(
                        {
                            output_log
                                .read_to_string(&mut output)
                                .expect("failed to read from file");

                            output.contains("should not run on init but on change")
                                && output.contains("Funzzy results")
                        },
                        "OUTPUT: {}",
                        output
                    );

                    // Concurrent test binaries write the same trigger file, so this child can
                    // fire extra runs beyond the one we triggered. Assert the
                    // shape of the FIRST run only: no init run, and the change
                    // fired the expected tasks in order.
                    let first_run = &output[..output.find("Funzzy results").unwrap()];
                    let first_run = setup::clean_output(first_run);
                    assert!(
                        !first_run.contains("Running on init commands"),
                        "must not run tasks on init: {}",
                        first_run
                    );
                    assert!(first_run.contains("Watching..."));
                    assert!(first_run.contains("running on init first"));
                    assert!(first_run.contains("should not run on init but on change"));
                    assert!(first_run.contains("run on init sencod"));
                },
            );

            Ok(())
        },
    )
    .expect("failed to run test");
}
