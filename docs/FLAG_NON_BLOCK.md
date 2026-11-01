# Busy-run policy (V2): wait and restart

Use `--on-busy wait` to finish active work before a new run. Use `--on-busy restart` to cancel it and run the newest change. V2 replaces the removed V1 `--non-block` flag with this explicit choice.

## Policy

```sh
fzz --on-busy wait      # default: finish the active run, then handle new work
fzz --on-busy restart   # cancel and reap active work, start the newest generation
fzz --restart           # convenience alias for --on-busy restart
```

- `wait` (default): the active run completes before a newer change is handled.
- `restart`: a newer change cancels and reaps all active tasks across every
  group (including descendants via process-group ownership), then starts the
  newest generation.
- Starting the control socket implies `--on-busy restart`.

## Migration from V1

| V1 | V2 |
| --- | --- |
| `fzz --non-block` / `fzz -n` | `fzz --on-busy restart` (or `--restart`) |
| (default behavior) | `fzz --on-busy wait` (default) |

Exit-code impact: none — policy affects which run completes, not the exit
contract (0 success, 1 failure, 2 usage).

See the [advanced guide](ADVANCED-GUIDE.md#1-parallel-execution) for failure and restart behavior. PARALLEL-EXECUTION-CONTRACT defines process ownership.
