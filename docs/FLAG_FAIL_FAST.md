# Stop after the first failure with `--fail-fast`

Use `--fail-fast` when later jobs should not run after a job fails. By default, Funzzy continues with the remaining jobs and reports the combined result.

```bash
fzz --fail-fast
```

This flag also cancels active jobs in a parallel group and skips jobs that have not started. It does not undo work that a job already completed. `FUNZZY_BAIL` is the environment equivalent.

## Example

```yaml
on:
  change: "directory/**/*"

jobs:
  - name: prepare
    run: echo ready
  - name: check
    run: exit 1
  - name: package
    run: echo packaged
```

Without `--fail-fast`, `package` runs after `check` fails. The overall run still fails. With `--fail-fast`, Funzzy skips `package` and reports the failure. If `prepare` and `check` were active in the same parallel group, Funzzy would cancel the active sibling after the first failure.

For behavior-level tests, see `tests/watching_with_fail_fast_flag.rs`. This flag was introduced in version 1.4.0. See [parallel failures and restart](ADVANCED-GUIDE.md#failures-and-restart) for V2 behavior.
