# Funzzy Managed Service Tasks Contract

> Status: **normative** — defined by TASK-0035. Managed long-running service
> tasks: explicit task kind, not a command that accidentally never exits.
> Process ownership per TASK-0030; output policies per TASK-0041.

## 1. Service kind

A job declares `service: true` to be managed as a long-running process:

```yaml
jobs:
  - name: dev-server
    service: true
    run: "vite dev"
    change: "src/**"
```

- A service is **started and kept running**; it is not treated as finite work
  that must exit.
- Services are **opt-in** (`service: true`). Finite jobs keep today's
  semantics exactly (default `service: false`).
- Services are **not hot code reload**: a change triggers the same
  rebuild/prep ordering as any job (see §4); the service itself is not
  auto-reloaded mid-flight beyond the defined restart policy.

## 2. Lifecycle states

| State | Meaning |
| --- | --- |
| `starting` | process spawned, not yet ready |
| `running` | process alive and (when probed) ready |
| `failed` | process exited unexpectedly and could not restart |
| `stopping` | graceful shutdown signal sent, grace pending |
| `stopped` | process fully reaped |

- **Start success**: the spawn succeeded (process group created, TASK-0030).
- **Readiness**: this revision defines readiness as *spawned + running* (the
  process is alive). An explicit readiness probe (e.g. HTTP) is out of scope.
- **Unexpected exit**: the service exited with a non-zero status while it was
  expected to keep running → `failed` (restart policy below).
- **Restart**: on unexpected exit, the service is restarted (bounded) unless
  the run is shutting down or a newer generation replaced it.

## 3. Restart and failure policy

- On unexpected exit, the service restarts with a bounded backoff (default:
  up to 3 attempts, 500ms between). Exceeding the bound → `failed`.
- A deliberate stop (watcher shutdown, cancel, replacement) is not a failure
  and does not count toward the bound.
- `--on-busy restart`: a newer generation cancels and reaps the service, then
  the new generation starts its own service.

## 4. Generation ordering

- Change → the matching generation's **prep commands run first** (finite
  jobs in the same generation), then the service (re)starts.
- The service is started after the generation's finite work is scheduled;
  a service never holds a finite-stage barrier.
- On watcher shutdown: all services receive the graceful signal, wait for
  grace, then escalate (TASK-0030), exactly like finite children.

## 5. Parallel interaction

- A service belongs to a generation like any job. A parallel group containing
  a service starts it as one member; the service does **not** hold the group
  barrier forever — the generation is considered scheduled once the service
  is spawned and running, and later groups proceed.
- Services are not included in "finite work completed" counts.

## 6. Status and control

- `status`/snapshots report a service's lifecycle state (`starting`,
  `running`, `failed`, `stopping`, `stopped`) per task, additive to the
  existing task fields.
- NDJSON run events (`--events`) carry a `service` flag on the task record.

## 7. Determinism and tests

- Deterministic tests use a fake service (a script that signals readiness
  then stays alive) and a fake clock where needed; integration covers real
  graceful and forced shutdown.
- Docs state services are opt-in and not hot code reload.

## 8. Out of scope

- Readiness probes (HTTP/port checks) — future revision.
- Auto-reload of service code (hot reload) — a change triggers the normal
  rebuild/prep ordering, not an in-place service reload.
- Supervisor-style dependency graphs between services.
