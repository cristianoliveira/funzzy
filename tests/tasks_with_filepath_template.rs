use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_replaces_filepath_template_with_changed_file() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_replaces_filepath_template_with_changed_file.log",
            example_file: "examples/jobs-with-filepath-template.yml",
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

            let path = fixture.join("examples/workdir/trigger-watcher.txt");
            let replaced = fixture.join("examples/workdir/foobar-watcher.txt");
            // Do not couple filepath expansion to the whole human result
            // boundary: job rows are additive and durations are intentionally
            // measured at runtime. Assert the stable command effects instead.
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains(&format!("this file has changed: {}", path.display()))
                        && output.contains("test_content")
                        && output.contains(&replaced.display().to_string())
                        && output
                            .matches("Funzzy results ----------------------------")
                            .count()
                            >= 2
                },
                "was not possible to observe filepath semantics: {}",
                output
            );

            let output = setup::strip_ansi_codes(&setup::clean_output(&output));
            assert!(output.contains(&format!("cat '{}'", path.display())));
            assert!(output.contains("Success; Completed: 3; Failed: 0;"));
        },
    );
}

#[test]
fn it_replaces_relative_path_relative_to_the_cunrrent_dir() {
    setup::with_example(
        setup::Options {
            output_file: "it_replaces_relative_path_relative_to_the_cunrrent_dir.log",
            example_file: "examples/jobs-with-filepath-template.yml",
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

            let absolute = fixture.join("examples/workdir/trigger-watcher.txt");
            // This fixture owns template expansion, not a byte-for-byte
            // rendering contract. Keep it resilient to additive job rows.
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains(&absolute.display().to_string())
                        && output.contains("examples/workdir/trigger-watcher.txt")
                        && output.contains(&format!(
                            "this is also valid: {} (nice!)",
                            absolute.display()
                        ))
                        && output.contains("this is invalid: {{ foobar }} (no!)")
                        && output
                            .matches("Funzzy results ----------------------------")
                            .count()
                            >= 2
                },
                "output: {}\nreason: filepath semantics did not settle",
                output
            );

            let output = setup::strip_ansi_codes(&setup::clean_output(&output));
            assert!(output.contains("Funzzy warning: Unknown template variable 'foobar'."));
            assert!(output.contains("Success; Completed: 4; Failed: 0;"));
        },
    );
}
