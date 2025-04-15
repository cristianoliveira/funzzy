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
This will create a `.watch.yaml` file in the current directory. Customize this file to define your Suggestion tasks.

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

### `-t` or `--target`

**Description**: Filter tasks by their target name. Runs only the tasks that match the given target name.

**Usage**:
```bash
fzz -t "@quick"
```

**Suggestion**: This is useful for running specific tasks without executing the entire workflow.

[More details](/docs/FLAG_TARGET.md)

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
