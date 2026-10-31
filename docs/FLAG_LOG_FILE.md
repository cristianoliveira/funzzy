# Save console output with `--log-file`

Use `--log-file FILE` to write Funzzy's output to a file as well as the console. The file is created or truncated when Funzzy starts. Its parent directory must already exist.

```bash
mkdir -p .tmp/funzzy
fzz --log-file .tmp/funzzy/watcher.log
```

Use `-l FILE` as a short form. Without `--log-file`, Funzzy writes output only to the console and creates no log file.

## Clear the log on configuration reload

Add `--log-truncate-on-change` to clear the current log when Funzzy reloads its configuration. This flag requires `--log-file`:

```bash
fzz --log-file .tmp/funzzy/watcher.log --log-truncate-on-change
```

A log file helps you inspect earlier command output. It does not replace the exact result and retained output that a control client can request by generation. See [control socket](ADVANCED-GUIDE.md#2-control-socket) for that workflow.

For behavior-level tests, see `tests/watching_with_log_file.rs`. `--log-file` was introduced in version 1.6.0.
