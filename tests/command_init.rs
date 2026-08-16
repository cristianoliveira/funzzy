use pretty_assertions::assert_eq;
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

            let file_content = std::fs::read_to_string(&file).expect("failed to read .watch.yml");
            // Deterministic comprehensive commented template (TASK-0093/0094):
            // small active starter + every supported optional property as a
            // comment, generated from the canonical option catalog.
            assert_eq!(
                file_content,
                "## Funzzy events file — .watch.yaml
# Comprehensive commented starter: small active setup, every supported
# option documented in comments. Full reference: `fzz config schema`.
#
# Next commands:
#   fzz check                  validate this file (no watcher)
#   fzz list                   show configured jobs
#   fzz run <name>             run one job once
#   fzz / fzz watch            start watching
#   fzz control status         talk to the running watcher
#   fzz config example minimal tiny machine-copyable starter

on:
  # change: Common change globs applied to every job (inherited, merged first)
  # change: \"**/*\"
  # ignore: Common ignore globs; explicit config ignore wins over gitignore
  # ignore: \"**/*.log\"
  # socket: Control socket path; enables the control surface (`fzz control`)
  socket: .tmp/funzzy/control.sock
  # concurrency: Global cap on simultaneously active tasks (default: machine parallelism)
  # concurrency: 2
  # debounce: Filesystem batch debounce window (default: 1s)
  # debounce: 500ms
  # watch_backend: Watch backend: native first, then poll when native is unavailable (default: auto) values: native | poll | auto
  # watch_backend: poll
  # poll_interval: Poll backend interval (only meaningful with `watch_backend: poll`) (default: 500ms)
  # poll_interval: 200ms
  # respect_gitignore: Respect workspace .gitignore rules (default: false)
  # respect_gitignore: true
  # success: Hook command run after a successful generation
  # success: echo ok > .fzz-success
  # failure: Hook command run after a failed generation
  # failure: echo failed > .fzz-failed
  # output: Default output policy for every job (default: inherit) values: inherit | quiet | capture | show-on-failure
  # output: quiet

jobs:
  # Optional job properties — uncomment the reference job to activate:
  #
  # name: Stable job identity; also the runtime task name. Must be unique
  # run: Command(s): a shell string or an argv list. Template variables: {{filepath}}, {{paths}}, {{relative_filepath}}
  # parallel: Named contiguous group; members run concurrently within the concurrency cap
  # cwd: Working directory for this job, relative to the workspace root
  # env: Per-job environment; values are never echoed or generated
  # service: Managed long-running service: started on init, restarted on change, stopped on exit (default: false)
  # output: Job output policy override; inherits on.output when absent values: inherit | quiet | capture | show-on-failure
  #
  # - name: reference
  #   run: [\"echo\", \"{{filepath}}\", \"{{paths}}\"]
  #   change: [\"**/*.rs\", \"**/*.md\"]
  #   ignore: [\"**/*.log\"]
  #   parallel: checks
  #   cwd: scripts
  #   env:
  #     FOO: bar
  #   service: true
  #   output: show-on-failure

  - name: hello world
    # run_on_init: Run this job when the watcher starts (default: false)
    run_on_init: true
    run: echo \"Funzzy hello world! Next step, add rules into .watch.yaml\"

  - name: list files
    # change: Globs that trigger this job (appended to on.change)
    change: '**/*.txt'
    # ignore: Globs that suppress a match; strongest precedence
    ignore: '**/*.log'
    run: 'ls -a'
",
                "file: {}",
                file_content
            );

            std::fs::remove_file(file)
                .expect("failed to remove file from examples/workdir/ignored");
        },
    );
}
