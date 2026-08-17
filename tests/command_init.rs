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
            // comment, generated from the canonical option catalog. Byte
            // stability is proven by command_init_proof (two runs compared);
            // the golden snapshot was removed with the docs-drift gate
            // (TASK-0110).
            let file_content = std::fs::read_to_string(&file).expect("failed to read .watch.yml");
            assert!(
                !file_content.trim().is_empty(),
                "init must write a template"
            );

            std::fs::remove_file(file)
                .expect("failed to remove file from examples/workdir/ignored");
        },
    );
}
