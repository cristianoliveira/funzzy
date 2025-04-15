## FLAG: `--non-block`

**Minimal version**: 1.5.0

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
Funzzy: Running on init commands.

Funzzy: make all

# (Changes detected)
```
The `make all` command will run and complete before restart.

### Example with the `--non-block` flag:

To illustrate the effect of `--non-block` let's use a `longtask.sh` script that simply counts from 0 to 5, with a 1 second delay between each number:

```bash
#!/bin/bash
TASK_NAME=$1
ITERATIONS=5

echo "Started task $TASK_NAME"
i=0
while [ $i -lt $ITERATIONS ]; do
  echo "Long task running... $i"
  sleep 1
  i=$((i + 1))
done
echo "Finished task $TASK_NAME"
```

And here is a `task-with-long-running-commands.yaml`:
```yaml
  - name: long task 2
    run: bash examples/longtask.sh long 2
    change: examples/workdir/trigger-watcher.txt

  - name: long task 1
    run: bash examples/longtask.sh long 1
    change: examples/workdir/trigger-watcher.txt
```

Including the `--non-block` flag gives immediate responsiveness:

```bash
$ fzz --config examples/tasks-with-long-running-commands.yaml --non-block
Funzzy: Running on init commands.

Funzzy: bash examples/longtask.sh long 2

Started task long 2
Long task running... 0
Long task running... 1
Long task running... 2
Long task running... 3 
(change detected)
^C # Simulate a file change by pressing Ctrl+C

Funzzy: bash examples/longtask.sh long 1

Started task long 1
Long task running... 0
Long task running... 1
Long task running... 2
Long task running... 3
Long task running... 4
Long task running... 5
Finished task long 1
```

### Tests

For a deeper understanding and to see this behavior in action, explore the tests located in `tests/watching_with_non_block_flag.rs`. These tests demonstrate the expected functionality when using the `--non-block` flag.

Enable `--non-block` when you prioritize rapid iteration and require an up-to-date test environment for each change.
