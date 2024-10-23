use std::collections::HashMap;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_when_using_fail_fast_exit_before() {
    setup::with_example(
        setup::Options {
            example_file: "examples/list-of-failing-commands.yml",
            output_file: "test_when_using_fail_fast_exit_before.log",
        },
        |fzz_cmd, mut output_file| {
            let mut output = String::new();
            let mut child = fzz_cmd
                .arg("--fail-fast")
                .spawn()
                .expect("failed to spawn sub process");
            defer!({
                child.kill().expect("failed to kill sub process");
            });

            wait_until!({
                output_file
                    .read_to_string(&mut output)
                    .expect("failed to read test output file");

                output.contains("Funzzy results") && output.contains("Failed tasks: 1")
            });

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s"
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!({
                output_file
                    .read_to_string(&mut output)
                    .expect("failed to read test output file");

                output.match_indices("Funzzy results").count() == 2
            });

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s
[2J
Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo before 

before

Funzzy: exit 1 

Funzzy results ----------------------------
Failed tasks: 1
 - Command exit 1 has failed with exit status: 1
Funzzy: finished in 0.0s",
                "failed to match ouput: {}",
                output
            );
        },
    );
}

#[test]
fn test_when_using_fail_fast_exit_before_with_env() {
    setup::with_env(
        HashMap::from([("FUNZZY_BAIL".to_string(), "1".to_string())]),
        || {
            setup::with_example(
                setup::Options {
                    example_file: "examples/list-of-failing-commands.yml",
                    output_file: "test_when_using_fail_fast_exit_before_with_env.log",
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

                        output.contains("Funzzy results") && output.contains("Failed tasks: 1")
                    });

                    assert_eq!(
                        setup::clean_output(&output),
                        "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s"
                    );

                    write_to_file!("examples/workdir/trigger-watcher.txt");

                    wait_until!({
                        output_file
                            .read_to_string(&mut output)
                            .expect("failed to read test output file");

                        output.match_indices("Funzzy results").count() == 2
                    });

                    assert_eq!(
                        setup::clean_output(&output),
                        "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s
[2J
Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo before 

before

Funzzy: exit 1 

Funzzy results ----------------------------
Failed tasks: 1
 - Command exit 1 has failed with exit status: 1
Funzzy: finished in 0.0s",
                        "failed to match ouput: {}",
                        output
                    );
                },
            );

            Ok(())
        },
    )
    .expect("failed to run test with env");
}

#[test]
fn test_fail_fast_with_non_block() {
    setup::with_example(
        setup::Options {
            example_file: "examples/list-of-failing-commands.yml",
            output_file: "test_fail_fast_with_non_block.log",
        },
        |fzz_cmd, mut output_file| {
            let mut output = String::new();
            let mut child = fzz_cmd
                .arg("-nb") // --non-block + --fail-fast
                .spawn()
                .expect("failed to spawn sub process");
            defer!({
                child.kill().expect("failed to kill sub process");
            });

            wait_until!({
                output_file
                    .read_to_string(&mut output)
                    .expect("failed to read test output file");

                output.contains("Funzzy results") && output.contains("Failed tasks: 1")
            });

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s"
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!({
                output_file
                    .read_to_string(&mut output)
                    .expect("failed to read test output file");

                output.match_indices("Funzzy results").count() == 2
            });

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: cat baz/bar/foo 

Funzzy results ----------------------------
Failed tasks: 1
 - Command cat baz/bar/foo has failed with exit status: 1
Funzzy: finished in 0.0s
[2J
Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo before 

before

Funzzy: exit 1 

Funzzy results ----------------------------
Failed tasks: 1
 - Command exit 1 has failed with exit status: 1
Funzzy: finished in 0.0s",
                "failed to match ouput: {}",
                output
            );
        },
    );
}
