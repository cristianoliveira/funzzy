#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_creates_the_config_file_with_cmd_init() {
    setup::with_output(
        "test_it_creates_the_config_file_with_cmd_init.log",
        |fzz_cmd, _| {
            std::env::set_current_dir("examples/workdir/ignored").expect("failed to change dir");

            fzz_cmd.arg("init").output().expect("failed to run init");

            let dir = std::env::current_dir().expect("failed to get current dir");
            let file = dir.join(".watch.yaml");
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

- name: run my test
  run: 'ls -a'
  run_on_init: true
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
