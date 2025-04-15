## FLAG: `--non-block`

The `--non-block` flag provides real-time responsiveness by ensuring that tasks are restarted immediately when a file change is detected. This feature prevents blocking or waiting for a current task to complete by sending a SIGTERM to the running process.

## USAGE

By default, the watcher waits for the current task to finish before initiating a restart on detecting a file change. This ensures that ongoing processes complete before handling new execution requests.

Activating the `--non-block` flag changes this behavior. Once a file change is detected, any ongoing process is terminated, and the tasks restart right away. This is particularly useful during development when you need rapid feedback and don't require the completion of current processes.

### Example of default behavior without the `--non-block` flag:

```yaml
- name: compile code
  run: "make all"
  change: "src/**/*.c"

- name: run tests
  run: "npm test"
  change: "tests/**/*.js"
```

Running with default settings:

```bash
$ fzz
Funzzy: Watching for changes...
// Change occurs and lets current task finish before restarting

Funzzy: make all
```

### Example with the `--non-block` flag:

Including the `--non-block` flag gives immediate responsiveness:

```bash
$ fzz --non-block
Funzzy: Watching for changes...
// Change detected and tasks restart immediately, canceling the current process if any

Funzzy: make all
```

### Environment Configuration

Alternatively, you can set the environment variable for similar behavior:

```bash
export FUNZZY_NON_BLOCK=true
$ fzz
```

By setting up the environment variable, `FUNZZY_NON_BLOCK`, you enable the non-blocking behavior without adding the flag to each command execution.

### Tests

For a deeper understanding and to see this behavior in action, explore the tests located in `tests/watching_with_non_block_flag.rs`. These tests demonstrate the expected functionality when using the `--non-block` flag.

Enable `--non-block` when you prioritize rapid iteration and require an up-to-date test environment for each change.
