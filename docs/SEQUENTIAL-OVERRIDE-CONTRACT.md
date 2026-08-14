# Funzzy Sequential Debugging Override Contract

> Status: **normative** — defined by TASK-0071. TASK-0072 implements local
> run/watch scope; TASK-0073 implements the exact control-generation scope,
> capability advertisement, and correlated snapshot concurrency fields.
> TASK-0074 proves the parallel-versus-sequential diagnostic workflow.
> Source research: `.tmp/reports/14-08-26/sequential-debug-override-recommendation.md`, `.tmp/reports/14-08-26/sequential-debug-override-plan.md`, `docs/AGENT-FEEDBACK-CONTRACT.md`, `docs/PARALLEL-EXECUTION-CONTRACT.md`.

When a parallel target fails nondeterministically, users and agents need an
exact comparison run with scheduler concurrency disabled. Editing
`.watch.yaml` to do this would reload the watcher, alter freshness, and
contaminate the diagnosis. This contract defines the explicit `--sequential`
override: the scheduler runs with effective concurrency exactly one, nothing
else about the run changes, and the comparison stays honest.

The canonical surface is `--sequential`, not a general `--jobs N` tuner. It
names diagnostic intent and avoids committing to per-run resource tuning
before real need exists.

## §1 Surface

```sh
fzz run @checks --sequential          # local one-shot run
fzz watch @checks --sequential        # this watcher session, all native generations
fzz ctl run @checks --sequential --wait --timeout 2m   # one exact control generation
```

`control` remains canonical; `ctl` is its visible alias (TASK-0070). The
override flag name is identical across local run, watch session, and control
run so one vocabulary covers all three scopes.

## §2 Scope semantics

| Scope | What `--sequential` affects | What it never affects |
| --- | --- | --- |
| Local `run` | Only the requested invocation; process exits after it. | Config file, other processes, watcher state. |
| `watch` session | Every generation scheduled by this watcher process, from start to exit. | Other watcher instances, the config file. |
| Control `run` | Only the exact scheduled generation; the request is answered when that generation reaches terminal outcome (or is superseded/cancelled/timeouts). | Later native generations; watcher-global policy. |

Invariants for every scope:

- Effective task concurrency is exactly `1`. Parallel stages still exist in
  the run plan; they execute one task at a time in deterministic order.
- Task selection, stage barriers, commands, cwd/env, templates, fail-fast,
  process ownership, streams, and timeout stay identical to the configured run.
- `.watch.yaml` is never rewritten.
- No automatic retry of side-effecting commands. Sequential mode is for a
  deliberate, explicitly requested comparison run — never an implicit
  fallback and never a silent re-execution.

## §3 Precedence and identity

- The override applies as **effective concurrency 1 over configured
  concurrency**: selection and topology come from config, execution
  concurrency from the override.
- For a control generation, the override applies to that exact generation
  only. Later native generations retain configured concurrency unless the
  watcher process itself was started with `--sequential`.
- `configured concurrency` and `effective concurrency` are reported
  separately, along with the override source (`cli` for local run/watch,
  `control` for an exact-generation request). Identity, freshness, and
  generation numbering semantics from AGENT-FEEDBACK-CONTRACT §1 are
  unchanged.

## §4 Execution signature and duration history

- The execution signature already includes the `jobs` concurrency parameter
  (see `plan.rs` `execution_signature(jobs, fail_fast)`). Effective
  concurrency flows into the signature the same way, so a sequential run and
  a parallel run of the same plan **never share duration history**.
- Sequential estimates therefore cannot contaminate parallel estimates and
  vice versa. This is what makes the diagnostic comparison meaningful: each
  mode compares against its own history.

## §5 Capability advertisement

- `capabilities.features` gains `sequentialOverride: true` only when the
  server actually implements the exact-generation override.
- A client that reads `sequentialOverride: false` (or an absent field) must
  treat a sequential control request as **unsupported**: the server rejects
  it explicitly. It never silently runs the generation in parallel — silent
  parallel execution after a sequential request would invalidate the
  comparison and is forbidden.
- The capability is static negotiation fact, like the other feature flags in
  `capabilities_result`; it performs no config reload or filesystem scan.

## §6 Correlation and output

- The snapshot/result for a controlled generation carries configured
  concurrency, effective concurrency, and override source without changing
  existing identity/freshness semantics.
- Structured output keeps the same `outputFormats` (`toon`, `json`) and adds
  the concurrency fields; raw command output still stays on the watcher side.

## §7 Exit codes

| Path | Exit code |
| --- | --- |
| Accepted: override scheduled/executed | `0` (same as the equivalent non-override success) |
| Unsupported: server lacks `sequentialOverride` | explicit error, non-zero, never silent parallel |
| Malformed/conflicting: invalid flag combination | Clap usage error `2` |
| Superseded: generation replaced before terminal | same superseded semantics as normal control run |
| Cancelled | same as normal control cancellation |
| Failed task(s) | same as normal run failure |
| Timeout on `--wait` | same as normal control timeout |

## §8 Agent diagnosis rule

The diagnostic conclusion must be conservative:

- Parallel fail + sequential pass is **`parallel-sensitive` evidence** — it
  shows the failure depends on concurrency.
- It is **never** a proof of a specific race root cause, stuck process, or
  data-race location.
- Sequential mode cannot remove races internal to one command, a managed
  service, the filesystem, or an external dependency. Those must be
  diagnosed separately.

## §9 Out of scope

- Generic `--jobs N` concurrency tuning: deferred until real need appears.
- Races inside one command/service/filesystem/external dependency.
- Automatic retry or fallback of any kind.
- Changing execution semantics beyond scheduler concurrency.
