# Funzzy Service Lifecycle Contract

Status: readiness settlement and worker-owned pooling are implemented by
TASK-0161. External black-box proof is in
`tests/control_await.rs` (`30f5ff4`). This document supersedes the older
TASK-0133 description that treated every live service as generation-owned.

## 1. Service modes

A `service: true` job has one of two modes:

- **Legacy service:** no `readiness` block. It keeps the existing behavior: the
  service belongs to its generation, remains unbounded while alive, and keeps
  that generation running. It is still stopped/reaped by cancellation,
  supersession, reload replacement, or watcher shutdown according to the
  existing process-ownership rules.
- **Readiness-enabled service:** includes a readiness command. Once the
  service is healthy, Funzzy transfers it to the worker-owned managed-service
  pool. The generation can then settle while the service remains alive.

Readiness is opt-in. A readiness block is valid only on `service: true` jobs.
Configurations without it retain legacy semantics.

## 2. Configuration

```yaml
on:
  socket: .tmp/funzzy/control.sock

jobs:
  - name: api
    service: true
    run: cargo run -- --port 8080
    change: "src/**"
    readiness:
      run: curl --fail http://127.0.0.1:8080/health
      timeout: 30s
      interval: 500ms
```

Readiness fields are strict:

- `run` is one non-empty shell command. It runs with the service job's
  resolved shell, working directory, environment, and path-template context.
- `timeout` is required, positive, and no greater than 24 hours.
- `interval` is optional and defaults to `500ms`; it must be positive and no
  greater than `timeout`.
- Unknown, missing, null, or wrongly typed readiness fields are configuration
  errors. Readiness on a non-service job is also rejected.

The complete effective readiness policy contributes to revision identity and
readiness-enabled service signatures. The policy is frozen when a generation
is scheduled; a later reload cannot change an in-flight probe decision.

## 3. Readiness execution

After the service process spawns successfully, Funzzy starts at most one
readiness attempt at a time. A failed attempt must exit before the next retry;
the interval begins after that attempt exits. Exit code zero promotes the
service only when the service is still alive. Non-zero attempts retry until
success or the absolute timeout. A readiness-command spawn failure fails
immediately.

A timeout terminates and reaps an active readiness process group before the
service lifecycle becomes failed. A service that exits before readiness is a
pre-readiness failure, not a post-settlement restart. Service and readiness
processes use separate ownership and output channels; readiness stdin is
null, so a probe cannot consume watcher input.

Readiness attempts use a synthetic `<service>:readiness` capture. Failed
attempts are intermediate evidence, not separate generation task failures.
The service's resolved context and frozen policy are reused after a restart.

## 4. Generation settlement and ownership

A generation settles exactly once when its finite work is terminal and every
readiness-enabled service included by that generation is ready. At the
promotion boundary, the executor detaches the live service through an opaque
`ServiceHandoff`; the worker adopts it into the managed pool before publishing
the generation terminal result. `CompletedRun` exposes no live service or
process handle.

The generation result is immutable and primary. A settled `passed` generation
stays passed even if its pooled service later restarts, fails, or stops. A
post-settlement service event does not reopen the generation or rerun its
hooks. Generation hooks run at the generation settlement boundary, once.

A generation containing a legacy service remains running while that service is
alive. This distinction is intentional: readiness is the explicit health
contract that permits settlement.

## 5. Pooled service lifecycle

The worker pool is keyed by service name and retains internal revision,
signature, origin-generation, instance, and lifecycle metadata. External
status intentionally projects only the secondary shape:

```json
"services": [
  {"name": "api", "state": "ready"}
]
```

The current generation outcome and this service projection are separate facts.
For example, status may show `state: "passed"` for the generation and
`services: [{"name":"api","state":"failed"}]` after a later service
failure.

A promoted service follows these post-settlement rules:

- zero exit is deliberate `stopped`; it is not restarted;
- non-zero exit enters `restarting`, consumes the existing bounded restart
  policy (three attempts with 500ms backoff), and must pass the frozen
  readiness probe again;
- exhausted restart/readiness attempts become `failed`;
- service lifecycle changes do not change generation history or generation
  hooks.

Replacement and reload decisions are worker-owned. A same-name replacement
reserves the current internal instance, physically stops and reaps the opaque
old handoff, and only then authorizes the new start. A stale revision,
instance, name, or signature fact cannot act on a newer pool entry. A
cancellation after the old reap suppresses the replacement start. Removed
services remain visible internally until their owned processes are reaped.

Omitted services remain pooled and untouched. Added or changed reload
services use the same stop/reap-before-start rule. Legacy `ReconcileServices`
and `StartServices` behavior remains available for existing callers.

## 6. Local runs and shutdown

A local one-shot `fzz run` has no worker pool. It probes readiness, computes
and publishes the frozen generation outcome, then deliberately stops and
reaps the service before returning. A watcher transfers a ready service to
its worker pool instead.

Watcher shutdown stops new work, cancels active probes, shuts down pooled
services, and waits for owned child handles to be reaped before crossing the
close boundary. Process-group signaling without direct-child reap is not a
successful shutdown proof.

## 7. Duration and retained evidence

For a readiness-enabled service, the service task's duration covers service
spawn through committed readiness and is recorded with the settled generation.
Later service uptime is pool state, not a completed generation duration. A
legacy live service has no completed lifetime and therefore no finite duration
row while it remains alive.

Generation and finite-job durations remain monotonic measurements supplied by
the executor. Renderers do not invent uptime, sum parallel job durations, or
replace a supplied `durationMs` value. Readiness output freezes with the
settled generation; later service output is not appended to that exact
-generation evidence.

See `CURRENT-RUN-JOB-DURATION-REPORT-CONTRACT.md` for duration formatting.

## 8. Observable output

Control status, await, and subscriptions keep the generation result as the
primary observation and expose the additive minimal `services` array as the
secondary live view. When no managed services exist, it is `[]`.

The MVP intentionally omits managed-service lifecycle records from the local
NDJSON event stream. Run, task, and generation records retain their existing
schema and meanings; service state is consumed through the status/control
projection. This omission is intentional and is not a promise of a rich
NDJSON service-log API.

## 9. Compatibility and proof

Finite jobs, recovery, timeouts, sequential/parallel scheduling, hooks, and
legacy service configurations remain unchanged. No implicit HTTP/TCP probe or
provider integration is added; the readiness command is the user's shell
command.

Black-box proof:

- `readiness_service_settles_generation_and_remains_in_pool` starts a
  readiness-enabled `jobs:` service, synchronizes on its start marker, proves
  exact await `passed`, status `api: ready`, and service liveness.
- `readiness_timeout_fails_generation_and_reaps_service` proves a failed
  readiness timeout returns a failed generation and reaps the service.

Focused command:

```sh
cargo test --test control_await --features test-integration readiness -- --nocapture
```

The broader worker/executor lifecycle tests cover deterministic probing,
restarts, stale facts, handoff ownership, pool actions, and shutdown. Do not
infer external replacement or reload behavior from those unit tests alone;
those paths require their own synchronized proof when changed.
