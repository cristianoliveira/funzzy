use std::process::Command;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_creates_the_config_file_with_cmd_init() {
    let dir = std::env::current_dir().expect("error getting current directory");
    let bin_path = dir.join("target/debug/fzz");

    std::env::set_current_dir(dir.join("examples/workdir/ignored"))
        .expect("failed to change to examples/workdir/ignored");

    Command::new(bin_path).arg("init").output().expect("failed to run init");

    let file = dir.join("examples/workdir/ignored/.watch.yaml");
    wait_until!(
        {
            // check if the .watch.yml file exists in examples/workdir
            println!("checking if file exists: {}", file.display());
            std::path::Path::new(&file).exists()
        },
        "the .watch.yml file was not created"
    );

    let file_content = std::fs::read_to_string(&file).expect("failed to read .watch.yml");
    assert_eq!(file_content, "
## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# list here all the events and the commands that it should execute

- name: run my test
  run: 'ls -a'
  change: 'src/**'
");

    std::fs::remove_file(file).expect("failed to remove file from examples/workdir/ignored");
}
