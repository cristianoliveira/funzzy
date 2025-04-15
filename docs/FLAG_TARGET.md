## FLAG: `--target <text_to_search>` **EXPERIMENTAL**

The `--target` flag allows you to execute specific tasks based on their names or @tags containing the specified `<text_to_search>`. By providing this flag, you can filter the tasks to be run, ensuring only those matching the target criteria are executed.

## USAGE

Given the following config file:

```yaml
- name: run my test @quick
  run: "echo 'quick tests'"
  change: "examples/workdir/**/*.yaml"

- name: run my build
  run: "echo 'building project'"
  change: "examples/workdir/**/*.rs"

- name: run my lint @quick
  run: "echo 'quick lint'"
  change: "examples/workdir/**/*.py"
```

### Example without the `--target` flag:

Running the `fzz` command without any target will execute all tasks:

```bash
$ fzz
Funzzy: Executing all tasks...
// run all tasks

Funzzy: echo 'quick tests'

quick tests

Funzzy: echo 'building project'

building project

Funzzy: echo 'quick lint'

quick lint
Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Duration: 0.0000s
```

### Example with the `--target` flag:

Running the `fzz` command using the `--target @quick` flag will filter tasks that are tagged with `@quick`:

```bash
$ fzz --target @quick
Funzzy: Filtering tasks with target '@quick'...
// run filtered tasks

Funzzy: echo 'quick tests'

quick tests

Funzzy: echo 'quick lint'

quick lint
Funzzy results ----------------------------
Success; Completed: 2; Failed: 0; Duration: 0.0000s
```

If no tasks match the provided `<text_to_search>`, a list of available tasks will be displayed for reference.

### Tests

Consider reviewing the tests in `tests/watching_filtered_tasks_with_target_flag.rs` for more detailed examples.

