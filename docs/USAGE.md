# Usage Guide for Funzzy

`funzzy` is a lightweight and blazingly fast file watcher. It allows you to define workflows that react to file changes, run commands, and manage tasks efficiently.

Alias: `fzz`

---

## Getting Started

### 1. Initialize Boilerplate
Create a boilerplate configuration file with:
```bash
fzz init
```
This will create a `.watch.yaml` file in the current directory. Customize this file to define your tasks.

To migrate a legacy configuration whose root is a task list:
```bash
fzz init --migrate
```
This wraps the existing list under `tasks:` while preserving task content and comments.

### 2. Running the Watcher
Start watching files and executing tasks with:
```bash
fzz
```

### 3. The configuration
Edit the `.watch.yaml` file to add or modify tasks. Each task can specify commands to run, files to watch, and ignore patterns. See the explanation below.

---

## Configuration File

The `.watch.yaml` file defines tasks and their triggers. Below is a sample configuration:

```yaml
- name: run commands on file change
  run: ["echo first", "echo second", "echo complex | sed s/complex/third/g"]
  change: "examples/workdir/trigger-watcher.txt"

# Explanation of the fields
# ----
# A description of the task
- name: task with ignoring rules
# Commands to execute when the task is triggered.
  run: "echo 'should not trigger when modifying ignored files'"
# One or more files or directories to watch for changes. Use glob patterns.
  change: "examples/workdir/**/*"
# One or more patterns to exclude from triggering the task. Use glob patterns.
  ignore:
    - "examples/workdir/ignored/**/*.txt"
    - "examples/workdir/another_ignored_file.foo"
# Indicate tasks that should execute when the watcher starts.
  run_on_init: false
```

### Common Rules Format

**Minimal version**: v1.6.0

You can reduce duplication by using the `on` section to define common `change` and `ignore` patterns shared across multiple tasks:

```yaml
# Common rules shared by all tasks
on:
  change:
    - "src/**"
    - "lib/**"
  ignore:
    - "**/*.log"
    - "**/*.tmp"

# Individual tasks that inherit common rules
tasks:
  # This task inherits all common rules
  - name: build
    run: cargo build

  # This task extends the common 'change' patterns
  - name: test
    run: cargo test
    change: "tests/**"

  # This task extends both 'change' and 'ignore'
  - name: lint
    run: cargo clippy
    change: "src/**/*.rs"
    ignore: "**/*.bak"
```

**Key Points:**
- **Backward Compatible**: The classic array format still works perfectly
- **Optional `on` Section**: You can omit it if you don't need common rules
- **Merge Semantics**: Task-specific patterns are merged with (added to) the common ones, so root-level scope and safety rails always apply
- **Flexible**: Each task inherits the common rules and can extend them

See [examples/common-rules.yml](../examples/common-rules.yml) for a complete example.

### Nested Groups Format

**Minimal version**: v1.6.0

When you have distinct areas of your project that watch different files (e.g., frontend, backend, docs), you can use nested groups to organize related tasks together. Each group can have its own set of common rules:

```yaml
# Frontend tasks watching frontend-specific files
- on:
    change:
      - "src/frontend/**"
      - "public/**"
    ignore:
      - "**/*.log"
  tasks:
    - name: frontend-build
      run: npm run build
    - name: frontend-test
      run: npm test

# Backend tasks watching backend-specific files
- on:
    change:
      - "src/backend/**"
      - "api/**"
    ignore:
      - "target/**"
  tasks:
    - name: backend-build
      run: cargo build
    - name: backend-test
      run: cargo test

# You can still mix in regular tasks
- name: regular-task
  run: echo "I'm not in a group"
  change: "docs/**"
```

**When to use nested groups:**
- You have distinct areas (frontend/backend/docs/config) that watch different files
- Different task groups need different ignore patterns
- You want better organization and separation of concerns
- Each group has multiple tasks that share the same watch patterns

**Key Points:**
- Each group is isolated - changes in one group don't affect others
- Tasks within a group extend the group-level `change` and `ignore` patterns
- You can mix groups and regular tasks in the same configuration
- Full backward compatibility maintained

See [examples/nested-groups.yml](../examples/nested-groups.yml) for a complete example.

---

## Flags and Options

### `-c` or `--config`
**Description**: Use a custom configuration file instead of the default `.watch.yaml`.

**Usage**:
```bash
fzz -c ~/path/to/custom-config.yaml
```

**Suggestion**: This is useful for running different workflows without modifying the default configuration.

---

### `-b` or `--fail-fast`
**Description**: Stops execution immediately if any task fails. This is useful when tasks are dependent on each other.

**Usage**:
```bash
fzz --fail-fast
```
**Suggestion**: This is useful for long-running tasks where you want to stop all tasks if one fails. Like e2e tests.

[More details](/docs/FLAG_FAIL_FAST.md)

---

### `list` and `watch TARGET`

**Description**: List configured tasks, or watch only tasks whose name or tag contains `TARGET`.

**Usage**:
```bash
fzz list
fzz watch "@quick"
```

**Suggestion**: List tasks first, then select a name or tag without executing the entire workflow.

---

### `explain PATH`

**Description**: Show which configured tasks a path would match or be ignored by, without starting a watcher or executing anything.

**Usage**:
```bash
fzz explain src/main.rs
fzz explain /absolute/path/to/file.rs
```

**Suggestion**: Use this to diagnose why a file change runs (or skips) a task. Matched tasks list the change rule; ignored tasks list both the change rule and the winning ignore rule. An unmatched path prints an explicit `unmatched` message.

---

### `exec`

**Description**: Watch stdin-supplied paths and run an ad-hoc program on each change. The child program and its arguments cross the CLI boundary without being joined and re-parsed through a shell.

**Usage**:
```bash
find . -name '*.rs' | fzz exec -- cargo fmt {{filepath}}
```

**Notes**:
- `--` marks the boundary between Funzzy options and the child command, so flag-like child arguments work (`fzz exec -- echo --help`).
- Arguments are preserved exactly: `fzz exec -- printf '<%s>\n' 'a b' c` passes `a b` as one argument.
- Shell operators (`|`, `&&`, globs) are not interpreted unless you explicitly invoke a shell: `fzz exec -- sh -c '...'`.
- A missing program is a usage error; a child non-zero exit is reported as a run failure while the watcher keeps watching.

---

### `-n` or `--non-block`
**Description**: Cancels currently running tasks when new changes are detected. Useful for workflows with long-running tasks.

**Usage**:
```bash
fzz --non-block
```
**Suggestion**: This is useful for tasks that take a long time to complete and many, allowing you to cancel them when new changes are detected.
The standard behavior is to wait for the current registered tasks to finish before starting new ones.

[More details](/docs/FLAG_NON_BLOCK.md)

---

### `-V` or `--verbose`
**Description**: Enables verbose mode to provide more detailed output about events and tasks.

**Usage**:
```bash
fzz -V
```

**Suggestion**: This is useful for debugging and understanding the flow of tasks and events.

---

### `--help`
- **Description**: Displays help information about the available commands and options.
- **Usage**:
  ```bash
  fzz --help
  ```

---

## Examples

Clone this repo to check examples

### Basic Example
Run a simple workflow:
```bash
fzz -c examples/simple-case.yml
```
Modify files in the `examples/workdir/` directory to see the output.

---

### Tasks with Failing Commands
Test workflows with intentionally failing tasks:
```bash
fzz -c examples/list-of-failing-commands.yml
```

---

### Long Running Tasks
Execute tasks with a long runtime using non-blocking mode:
```bash
fzz -c examples/reload-config-example.yml --non-block
```

---

### Run Tasks on Initialization
Use tasks that execute only on initialization:
```yaml
- name: cleanup before start
  run: "rm -rf temp/*"
  run_on_init: true
```

For additional examples, see the [examples folder](https://github.com/cristianoliveira/funzzy/tree/master/examples).
