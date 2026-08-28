---
id: TASK-0138
title: Define bounded finite-job execution timeout contract
status: todo
depends_on: [TASK-0134]
priority: normal
tags: [design, timeout, jobs, process, control-socket, determinism]
---

# Define bounded finite-job execution timeout contract

## Problem

A finite blocking command can remain running forever. Existing control `--timeout` bounds only the caller's await; it neither cancels nor terminates the job. Users need an optional execution deadline without confusing client patience with process lifetime.

## Preferred API to evaluate

```yaml
jobs:
  - name: await-remote
    timeout: 30m
    run: ./scripts/await-remote.sh
```

The field is generic finite-job policy, not an external-system feature. Absence should preserve today's unbounded runtime.

## Acceptance criteria

- [ ] Define `jobs[].timeout` syntax, positivity bounds, absence/default behavior, and canonical schema/help representation.
- [ ] Define the deadline start precisely (scheduled, spawning, or successfully spawned) and use monotonic elapsed time.
- [ ] Define deterministic precedence among natural exit, configured timeout, user cancellation, generation supersession, fail-fast sibling failure, and watcher shutdown.
- [ ] Define timeout outcome separately from command failure and client-await timeout across task state, generation terminal reason, human output, structured snapshots/events, retained evidence, and exit codes.
- [ ] Define full process-group termination, bounded graceful shutdown, escalation, reap, and duration accounting at timeout.
- [ ] Define output behavior so evidence produced before/during shutdown remains bounded and attributable.
- [ ] Define interactions with parallel groups, recovery, hooks, config reload, frozen generation revisions, and duration history.
- [ ] Reject or explicitly define timeout on `service: true`; do not silently reuse finite terminal semantics for managed services.
- [ ] Preserve current behavior for jobs without timeout and all legacy configuration forms unless explicitly included.
- [ ] Record additive control-protocol/capability and `pi-watcher` decoder impact before implementation.
- [ ] Require injected clock/deadline seams and synchronization-based tests; fixed sleeps are not an acceptable correctness strategy.

## Non-goals

- Client await deadline changes.
- Provider/API polling deadlines.
- A global default timeout.
- Remote approval or arbitrary cancellation policy.
