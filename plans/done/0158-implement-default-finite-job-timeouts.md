---
id: TASK-0158
title: Implement default finite-job timeouts
status: done
depends_on: [TASK-0157]
priority: high
tags: [rust, timeout, jobs, config, tdd]
---

# Implement default finite-job timeouts

## Problem
Once the default-timeout contract is approved, Funzzy must resolve and freeze each finite job's effective timeout without changing existing per-job timeout outcomes or legacy configurations.

## Context

Implement only the approved TASK-0157 contract. The feature resolves a default for each finite job; it does not add a generation timer or alter the executor's established timeout lifecycle.

## Acceptance criteria

- [ ] Write failing tests first for absent default, inherited default, per-job override, invalid values, service interaction, and any approved explicit opt-out syntax.
- [ ] Parse and validate the approved execution-level property through the preferred V2 configuration path without accepting it in legacy task shapes or unrelated sections.
- [ ] Resolve each finite job's effective timeout with the approved precedence before execution planning; preserve `None` as unbounded when neither value is configured.
- [ ] Carry the frozen effective value through the existing `Rules` and run-plan seams so local run, blocking watch, restart-capable watch, sequential, and parallel execution cannot diverge.
- [ ] Keep direct `jobs[].timeout` validation and `service: true` constraints consistent with TASK-0138; an execution default does not turn managed services into finite jobs.
- [ ] Include the new property/effective value in semantic revision identity so a default-only reload affects later generations and never mutates active work.
- [ ] Update the canonical option catalog, JSON Schema sections, generated init/example profiles, config validation, and reconstructed configuration output from one source of truth where possible.
- [ ] Preserve existing timeout precedence, process-group shutdown, typed `timedout` evidence, failure hooks, recovery exclusion, duration history, and client-await behavior.
- [ ] Keep configurations without the execution default behaviorally unchanged, including accepted legacy formats.
- [ ] Add or adjust control/pi-watcher code only if TASK-0157 identifies an observable wire change.

## Verification focus

Keep parsing, inheritance, effective-value, revision, and service cases in deterministic unit tests. Spawned process behavior belongs to TASK-0159; do not duplicate the already-proven process shutdown implementation.
