# Skip startup jobs with `--no-run-on-init`

Use `--no-run-on-init` to skip jobs marked `run_on_init: true` when a watcher starts. The jobs remain configured and can still run when files change or when you select them explicitly.

```bash
fzz --no-run-on-init
```

Without this flag, Funzzy runs jobs marked `run_on_init: true` at watcher startup. This flag does not filter jobs from the workflow.

## Example

```yaml
on:
  change: "examples/workdir/**/*"

jobs:
  - name: check
    run: echo checked
    run_on_init: true
```

Run `fzz` to start the watcher and run `check` at startup. Run `fzz --no-run-on-init` to start the watcher without that startup run. A later change under `examples/workdir/` can still run `check`.

For behavior-level tests, see `tests/ignore_run_on_init.rs`. This option was first documented for a nightly build.
