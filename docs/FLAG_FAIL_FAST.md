## FLAG: `--fail-fast`

The default behavior is to continue executing all tasks, regardless of their success or failure and report.

The `--fail-fast` flag allows you to stop the execution of tasks as soon as one fails. If any task exits with a non-zero status code, all subsequent tasks will be halted immediately. This option is useful when you want to avoid unnecessary task processing if a critical task fails. TIP: Use this to do the red-green-refactor 

## USAGE

Consider the following configuration file:

```yaml
- name: first task
  run: "echo 'This task will run'"
  change: "directory/**/*"

- name: failing task
  run: "exit 1"  # This task is intended to fail
  change: "directory/**/*"

- name: last task
  run: "echo 'This task will not run if --fail-fast is set'"
  change: "directory/**/*"
```

### Example without the `--fail-fast` flag

When you execute `fzz` without using the `--fail-fast` flag, all tasks will be executed regardless of their success or failure:

```bash
$ fzz
Funzzy: Running each command...

Funzzy: echo 'This task will run'
This task will run

Funzzy: exit 1


Funzzy: echo 'This task will not run if --fail-fast is set'
This task will not run if --fail-fast is set

Funzzy results ----------------------------
Failure; Completed: 2; Failed: 1; Duration: ...s
```

### Example with the `--fail-fast` flag

When the `--fail-fast` flag is used, task execution stops as soon as a failure is encountered:

```bash
$ fzz --fail-fast
Funzzy: Running each command...

Funzzy: echo 'This task will run'
This task will run

Funzzy: exit 1
Funzzy results ----------------------------
- Command exit 1 has failed with exit status: 1
Failure; Completed: 1; Failed: 1; Duration: ...s

# Notice the 'This task will not run if --fail-fast is set' was not echoed
```

Running the `fzz` command with the `--fail-fast` flag will ensure tasks stop on failure, optimizing task execution time and preventing unnecessary work.

### Tests

Refer to `tests/watching_with_fail_fast_flag.rs` for detailed test scenarios related to the `--fail-fast` functionality.

