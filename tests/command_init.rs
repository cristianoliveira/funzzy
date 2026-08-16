use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_creates_the_config_file_with_cmd_init() {
    setup::with_output(
        "test_it_creates_the_config_file_with_cmd_init.log",
        |fzz_cmd, mut output_file, fixture| {
            let dir = fixture.join("examples/workdir/ignored");
            fzz_cmd.current_dir(&dir);
            let file = dir.join(".watch.yaml");
            let _ = std::fs::remove_file(&file);

            fzz_cmd.arg("init").output().expect("failed to run init");

            let mut output = String::new();
            wait_until!(
                {
                    output_file
                        .read_to_string(&mut output)
                        .expect("failed to read test output file");

                    output.contains("Configuration file created successfully! To start run `fzz`")
                },
                "Unexpected outout: {}",
                output
            );

            wait_until!(
                {
                    // check if the .watch.yml file exists in examples/workdir
                    println!("checking if file exists: {}", file.display());
                    std::path::Path::new(&file).exists()
                },
                "the .watch.yml file was not created"
            );

            // Deterministic comprehensive commented template (TASK-0093/0094):
            // small active starter + every supported optional property as a
            // comment, generated from the canonical option catalog. The golden
            // bytes live in scripts/golden/init-template.yaml — the reviewed
            // snapshot (TASK-0095) and the source the drift gate diffs against.
            let file_content = std::fs::read_to_string(&file).expect("failed to read .watch.yml");
            let golden = std::fs::read_to_string(format!(
                "{}/scripts/golden/init-template.yaml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("failed to read golden init template");
            assert_eq!(file_content, golden, "file: {}", file_content);

            std::fs::remove_file(file)
                .expect("failed to remove file from examples/workdir/ignored");
        },
    );
}
