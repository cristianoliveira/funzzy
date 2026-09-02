---
id: TASK-0160
title: Define service readiness and settled-generation contract
status: done
depends_on: []
priority: high
tags: [design, services, readiness, generations, control-socket, determinism]
---

# Define service readiness and settled-generation contract

## Problem

A generation containing a healthy `service: true` job stays `running` for the service's entire lifetime. This makes a successfully started development pipeline look unfinished to humans and agents, even after every finite check has passed.

## Desired outcome

A watched generation reports terminal success once finite jobs pass and every included readiness-enabled service proves healthy. Ready services remain managed by the watcher, while their current lifecycle is reported separately from immutable generation history.

## Accepted configuration

```yaml
jobs:
  - name: api
    service: true
    run: cargo run
    readiness:
      run: curl --fail http://localhost:8080/health
      timeout: 30s
      interval: 500ms
```

`readiness` is allowed only when `service: true`.

- `run` is one non-empty shell command. It uses `$SHELL -c`, with `/bin/sh` as fallback. Argv readiness is a future additive surface.
- `timeout` is required, positive, and no greater than 24 hours.
- `interval` is optional, defaults to `500ms`, must be positive, and cannot exceed `timeout`.
- Unknown, missing, null, or wrongly typed fields fail `fzz check` with an actionable error.
- The field is accepted wherever the existing job/service shape is accepted. Configurations without it retain current behavior.
- Canonical schema, option catalog, generated init content, config rendering, migration-safe parsing, revision identity, and service signature include the complete readiness policy.

Successful process spawn or elapsed stabilization time is not application health. A service without `readiness` preserves current generation-owned, perpetually-running behavior; users opt into settled generations explicitly.

## Readiness execution

1. The absolute timeout starts after the service process spawns successfully.
2. Funzzy starts at most one readiness command at a time. A failed check never overlaps its retry; the interval begins after that attempt exits.
3. Exit code zero marks the service ready only if the service process is still alive when readiness is committed.
4. Nonzero check exit retries until success or the absolute timeout. Readiness-command spawn failure fails immediately.
5. If one check is still running at the absolute timeout, Funzzy terminates and reaps its complete process group before failing readiness.
6. Service spawn failure, any service exit before readiness, readiness timeout, or readiness-command spawn failure fails the service task and its generation. Pre-readiness service exit is not hidden by the normal post-readiness restart policy.
7. The service and every readiness attempt own distinct process groups. Cancellation, supersession, reload replacement, and shutdown signal the applicable groups, wait the existing bounded grace, escalate, wait/reap every direct child handle, join output forwarders, and confirm each owned process group is gone before transition completion. Funzzy does not claim to `wait*` non-child descendants; a subreaper/supervisor is out of scope.
8. The readiness command uses the service job's resolved `cwd`, `env`, and `{{filepath}}`/`{{paths}}` expansion. Its stdin is null so a probe cannot steal watcher input. It receives no implicit network or provider behavior.
9. Readiness attempts use a synthetic `<service>:readiness` output channel in the same generation and follow the service job's output policy. Service startup output remains under the configured service task. Both use the existing 1 MiB global retention budget, paging limits, and exact `control output --generation N` retrieval; nonzero attempts are intermediate evidence, not separate `tasks[]` failures. At promotion, both captures freeze. Later service output continues only through live/log-file forwarding.
10. Readiness duration is measured from service spawn through committed readiness. For a settled generation, that is the service task duration; later service uptime is separate live-service telemetry.

## Generation settlement and ownership

Readiness does not insert a new serial barrier. Existing plan order and parallel-group behavior remain unchanged: later stages may run while a service is starting. A generation can settle only after its finite work is terminal, every readiness-enabled service it included is ready, and it owns no legacy service without `readiness`.

When all finite work passes and all readiness-enabled services are ready, those services atomically transfer into one watcher-owned managed-service pool. The pool is keyed by configured service name and retains the service signature, configuration revision, origin generation, process-group ownership, restart budget, and current lifecycle state. In a mixed generation, readiness-enabled services still transfer after this promotion barrier, but any legacy service remains generation-owned and keeps that generation `running`; this preserves existing no-readiness behavior.

- The generation records each promoted service task as passed, meaning startup readiness passed. A generation that can settle emits exactly one terminal result and is never reopened.
- A later generation that omits that service leaves the ready pooled service running.
- A later generation that selects the service replaces it, preserving restart-by-re-inclusion semantics even when its config signature is unchanged. Selection alone does not stop the old instance.
- Replacement begins only when the service task becomes runnable at its exact configured serial position or parallel-group occurrence. The worker atomically reserves that service name and instance ID, emits `stopping` for the old instance, fully stops/reaps it, and only then authorizes the new spawn. Parallel siblings may proceed, but no second reservation or spawn for that name can pass the handshake. Same-name processes never overlap.
- Fail-fast skip, cancellation, or failure before reservation leaves the old instance running. Cancellation after reservation finishes reaping the old instance but suppresses the new spawn. Cancellation after spawn reaps the new starting instance. Neither path rolls back the old process.
- If replacement starts and then fails readiness, Funzzy does not resurrect the old process. The desired service entry becomes `failed` until later selection, reload, or watcher restart.
- Reload leaves unchanged pooled services running. Removed services reserve, stop, reap, then disappear. Added or signature-changed readiness services reserve the name, deliberately stop/reap the old instance first, then start and probe through pool reconciliation; replacement intentionally accepts downtime and has no rollback. A failed reload replacement remains in `services[]` as `failed` with `originGeneration: null`, the committed reload revision, and bounded error evidence. It does not rewrite an active or terminal generation.
- Readiness-enabled services selected by `fzz ctl run` use normal watcher ownership. `run` returns the scheduled generation immediately; exact await becomes terminal after finite work/readiness promotion, and subsequent status reports that terminal generation plus the pooled service.
- A local one-shot `fzz run` has no watcher pool: it probes readiness, freezes the generation result, deliberately stops and reaps the service, publishes the frozen terminal result, runs its terminal generation hook, and returns. Graceful or escalated successful reap does not change the result. Failure to confirm reap is a separate CLI infrastructure error and nonzero process exit; it does not change the frozen generation outcome or select the opposite hook.
- Watcher shutdown quiesces new work, cancels starting probes, sends an ordered pool-shutdown command, waits for every probe/service child handle to be reaped, and only then crosses the close-hook boundary. Process-global signaling alone is not sufficient proof of shutdown.

## Ownership and reconciliation state machine

The worker consumer owns one coordinator for readiness arbitration and the managed-service pool. It is the sole authority for the new service lifecycle states below. Existing executor `Started`, `TaskTerminal`, `Cancelled`, and `Finished` publication remains in place; however, the executor must request coordinator promotion approval before it may publish `Finished`, so service promotion commits before the immutable terminal event.

```text
absent
  -> generation-starting(instance, generation, revision)
  -> generation-ready(instance)
  -> pooled-ready(instance)
  -> pooled-restarting(instance)
  -> pooled-ready(instance) | failed(instance) | stopped(instance)

pooled-ready|restarting|failed|stopped
  -> reserved-replacement(old instance, desired revision)
  -> stopping(old instance)
  -> generation-starting(new instance)      # generation selection
     or pool-starting(new instance)          # reload reconciliation
  -> pooled-ready(new instance) | failed(new instance)

any owned state -> stopping -> absent         # removal/shutdown
```

Generation promotion is atomic: under the coordinator lock, every eligible `generation-ready` service is removed from the run, inserted into the pool, its frozen output is registered, and its ordered lifecycle transition is committed; only then may the existing executor path publish the generation terminal event. The lock is released before invoking event sinks: committed transitions enter a sequence-ordered emission queue, preventing re-entrant sinks while preserving service-before-generation ordering. Omitted services receive no transition. A selected replacement uses the reservation handshake above. A run that never reaches reservation cannot affect the pool.

Pool commands are explicit ordered worker commands: `ReserveServiceReplacement`, `ReconcileServicePool`, and `ShutdownServicePool`. Worker command acceptance order breaks ties, except shutdown/cancel/supersession priority defined below. Each command carries desired config revision and service signature. Reload commit first updates desired pool revision, then enqueues reconciliation. A queued generation start from an older revision is stale and cannot replace a service desired by the newer revision; a generation frozen after reload may do so. Concurrent reload, generation, and shutdown transitions therefore have one serial history.

## Post-settlement service lifecycle

A ready service may change state without changing generation history:

- Zero exit is a deliberate `stopped` service state and is not restarted.
- Nonzero exit enters `restarting` and preserves the existing bounded three-attempt, 500ms-backoff policy.
- Every restarted process must pass the same frozen readiness policy before returning to `ready`.
- Restart spawn/readiness failure consumes the current restart attempt. Exhaustion produces `failed` with bounded attributable error evidence.
- Post-settlement ready, restart, stop, or failure events never emit another generation result and never run generation success/failure hooks.

No automatic rollback to an earlier service instance is promised.

## Deterministic race precedence

Each worker observation cycle establishes one sequence marker under the coordinator lock, drains every command accepted before that marker, and then polls children/time. Work accepted after the marker belongs to the next cycle; wall-clock simultaneity outside this boundary has no meaning.

Within one cycle, the coordinator applies visible facts in this order:

1. watcher shutdown, explicit cancellation, supersession, or reload replacement command;
2. service process exit;
3. readiness absolute timeout;
4. readiness-command exit.

Therefore a cancellation accepted before the cycle marker wins over readiness in that cycle, service exit wins over probe success, and a success observed at or after the deadline loses to timeout. The implementation must add this command-arbitration boundary before calling executor advancement; the current advance-then-`try_recv` loop does not satisfy the contract. Tests use an injected clock/process runner and synchronized command/child barriers; fixed sleeps are not correctness evidence.

## Hooks

- Generation success runs once after all finite jobs pass and selected readiness-enabled services become ready.
- Any pre-readiness service failure contributes to the immutable failed generation and its one failure hook.
- Post-settlement service lifecycle changes are telemetry only and run no generation hook.
- Reload pool reconciliation runs no generation hook because it does not create or mutate a generation.
- Close-hook ordering follows watcher shutdown after starting probes and pooled services are reaped.

## Observable contract

Generation outcome and current service lifecycle are separate facts.

Control protocol `1.0` advances `schemaVersion` from `1` to `2`, adds `features.managedServices: true`, and lists `services` in `optionalFields` for cross-version discovery. Every schema-2 status/await/subscription snapshot contains `services`; when none exist it is `[]`, never absent or null.

`services` is the union of readiness-enabled jobs in the committed config and owned/draining readiness-enabled instances. It has one entry per name. During same-name replacement, the old `instanceId` remains that entry in `stopping`; Funzzy allocates and swaps in the new `starting` instance only after the old process group is confirmed gone, so two entries are unnecessary. A removed draining instance remains until reap. Current-config entries follow job declaration order, followed by removed draining entries in ascending `instanceId`. Every entry has exactly this camelCase shape:

```json
{
  "name": "api",
  "instanceId": 7,
  "state": "starting | ready | restarting | stopping | failed | stopped",
  "originGeneration": 12,
  "revision": 4,
  "signature": "sha256:...",
  "restartAttemptsUsed": 0,
  "restartAttemptsRemaining": 3,
  "startedAtEpochMs": 0,
  "readyAtEpochMs": null,
  "uptimeMs": null,
  "latestError": null
}
```

`instanceId` is watcher-instance-local, monotonic, and never reused. `originGeneration` is the selecting generation or null for reload-only starts. Timestamps/uptime are null until meaningful. `latestError` is null or a tail-truncated UTF-8 string bounded to 40 lines and 4 KiB. When truncated, the first retained line is exactly `[...truncated...]`; the marker counts toward both limits. Removed entries disappear only after direct-child reap and process-group disappearance are confirmed.

Every lifecycle transition emits one NDJSON event with event schema version `2` and `event: "service_lifecycle"`, plus watcher-local monotonic `sequence`, `tsMs`, `name`, `instanceId`, `state`, nullable `originGeneration`, `revision`, restart counts, and nullable bounded `latestError`. Transition state and sequence commit under the coordinator lock; sinks run after unlock in sequence order. Snapshot subscriptions publish the resulting full snapshot; they never publish a second generation terminal event. Exact-generation await returns when generation work/readiness gates settle and does not wait for pooled service lifetime.

Readiness-attempt output is retained with the starting generation. Once that generation settles, its retained output is immutable: later service stdout/stderr continues through normal live/log-file forwarding but is not appended to exact-generation output. A separate retained service-log API is out of scope; current service state keeps only bounded health/error evidence.

Human status and pi-watcher rendering must show both facts without collapsing them, for example: `generation 12: passed` and `api: ready`. A post-settlement failure renders `generation 12: passed` with `api: failed`, never a contradictory failed generation.

The wire change is additive: protocol version remains `1.0`, control schema becomes `2`, and NDJSON event schema becomes `2`. Existing clients that ignore unknown fields continue to read generation results. In the same delivery, update permissive decoding and rendering in `pi-watcher/src/domain/watcher.ts`, `pi-watcher/src/domain/capabilities.ts`, and `pi-watcher/src/infra/client.ts`, plus decoder, capability, status/await/subscription, golden-wire, and malformed-service fixtures. An old schema-1 server may omit `services`; pi-watcher normalizes that absence to `[]`. A schema-2 payload missing or nulling `services` is malformed.

## Compatibility

- Finite jobs, timeouts, recovery, output policies, hooks, and configurations without services are unchanged.
- Services without `readiness` keep current generation ownership, restart-by-re-inclusion, reload, and non-terminal generation behavior.
- Readiness-enabled services deliberately opt into watcher ownership after the promotion barrier.
- Readiness policy participates in semantic config revision and service signature only when present (`run`, resolved `timeout`, resolved `interval` in canonical order). The absent encoding preserves existing legacy-service signatures.
- Existing automatic restart constants remain unchanged.
- No implicit HTTP, TCP, container, or provider integration is introduced.

## Non-goals

- Treating spawn or elapsed sleep as application health.
- Built-in HTTP/TCP/container probes.
- Argv readiness commands in the first implementation.
- Hot reload inside a service process.
- Automatic rollback to a previous service instance.
- Generation failure hooks for post-settlement service degradation.
- Retained full service-log retrieval after generation settlement.
- Changing finite-job timeout semantics.

## Acceptance criteria

- [x] Finalize syntax, validation, defaults, shell support, and canonical configuration surfaces.
- [x] Define readiness success, retry, timeout, failure, command context, output, duration, and process ownership.
- [x] Define settlement for sequential/parallel finite work and pre-readiness service failure.
- [x] Define watcher ownership, omission, re-inclusion, replacement, reload, local run, cancellation, and shutdown.
- [x] Define immutable generation history and post-readiness stop/restart/failure semantics.
- [x] Separate generation outcome from current service state across status, events, evidence, duration, and pi-watcher.
- [x] Define generation and close-hook boundaries.
- [x] Define deterministic race precedence and synchronization-based proof requirements.
- [x] Record compatibility and coordinated control/pi-watcher impact.

## Outcome

Accepted product direction: explicit command readiness opts a service into settled-generation and watcher-owned lifecycle semantics. Implementation proceeds through TASK-0161, followed by black-box proof and documentation in TASK-0162.

Lead approved the product, compatibility, ownership, and wire contract. Developer feasibility review approved the final replacement handshake, Unix cleanup guarantee, coordinator boundary, reload lifecycle, local-run behavior, frozen evidence, and deterministic race semantics. QA review remains queued for TASK-0161/0162 acceptance proof.

## Related work

- TASK-0035 introduced managed service jobs.
- TASK-0133 documented current generation-owned service behavior and its init-only limitation.
- TASK-0161 implements this contract.
- TASK-0162 proves and documents it at external boundaries.
