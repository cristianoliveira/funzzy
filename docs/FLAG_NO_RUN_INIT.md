## FLAG: `--no-run-on-init` 

**Minimal version**: nightly

Tasks configured with `run_on_init: true` are executed when the watcher starts. By default, this behavior is enabled.
By using the `--no-run-on-init` flag, you can disable this behavior and prevent tasks from running on initialization.
The remaining triggers will still work as expected. It does not filter the tasks.

## USAGE

Given the following config file:
```yaml
- name: a task that does not run on init
  run: "echo 'should not run on init'"
  change: "examples/workdir/**/*"
  ignore:
    - "examples/workdir/ignored/**/*.txt"
    - "examples/workdir/another_ignored_file.foo"

- name: a task that runs on init
  run: "echo 'should run on init'"
  change: "examples/workdir/**/*"
  ignore:
    - "examples/workdir/ignored/**/*.txt"
    - "examples/workdir/another_ignored_file.foo"
  run_on_init: true
```

When running `fzz` without the `--no-run-on-init` flag, the output will be:

```bash
$ fzz
Funzzy: Running on init commands.
// run tasks...

```

When running `fzz` with the `--no-run-on-init` flag, the output will be:

```bash
$ fzz --no-run-on-init
Funzzy: Watching...
```

The remaingin triggers work as expected, so if you change the file `examples/workdir/trigger-watcher.txt` the task will run.

### Tests

Check the tests in `tests/ignore_run_on_init.rs` for more details.
