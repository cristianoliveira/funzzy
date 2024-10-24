use pretty_assertions::assert_eq;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_creates_the_config_file_with_cmd_init() {
    setup::with_output(
        "test_it_creates_the_config_file_with_cmd_init.log",
        |fzz_cmd, mut output_file| {
            std::env::set_current_dir("examples/workdir/ignored").expect("failed to change dir");

            let dir = std::env::current_dir().expect("failed to get current dir");
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

            let file_content = std::fs::read_to_string(&file).expect("failed to read .watch.yml");
            assert_eq!(
                file_content,
                "## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# List here the tasks and the commands for this workflow
# then run `fzz` to start to work.

- name: hello world
  run: echo \"Funzzy hello world! Next step, add rules into .watch.yaml\"
  run_on_init: true

- name: list files
  run: 'ls -a'
  change: '**/*.txt'
  ignore: '**/*.log'
",
                "file: {}",
                file_content
            );

            std::fs::remove_file(file)
                .expect("failed to remove file from examples/workdir/ignored");
        },
    );
}
