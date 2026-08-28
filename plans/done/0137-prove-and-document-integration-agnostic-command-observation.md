---
id: TASK-0137
title: Prove and document integration-agnostic command observation
status: todo
depends_on: [TASK-0136]
priority: high
tags: [integration-tests, docs, jobs, control-socket, axi, reliability]
---

# Prove and document integration-agnostic command observation

## Problem

Users need executable guidance showing how a blocking script can represent any external result without coupling Funzzy to providers or repurposing managed-service restart behavior.

## User workflow

```yaml
on:
  socket: .tmp/funzzy/control.sock

jobs:
  - name: await-remote
    trigger: manual
    run: ./scripts/await-remote.sh
```

Foreground composition:

```sh
git push && fzz run await-remote
```

Running-watcher composition:

```sh
git push && fzz ctl run await-remote --wait --timeout 31m --format toon
```

The control `--timeout` example is an await deadline only. Until TASK-0138 through TASK-0140 land, the script owns any execution deadline.

## Acceptance criteria

- [ ] Add black-box proof using a deterministic local blocking script; no live external APIs, credentials, or sleep-based assertions.
- [ ] Prove alive → running, exit `0` → passed once, and non-zero → failed once with bounded evidence.
- [ ] Prove exact control `runId` works with existing await, output, and cancel APIs and remains isolated from later generations.
- [ ] Prove a manual-only target does not run on initialization or matching filesystem changes, including when root `on.change` is present.
- [ ] Prove local foreground composition exits with the script result and a failed preceding command does not start the target.
- [ ] Document the script boundary: it owns authentication, polling, correlation, retries, and provider semantics; Funzzy owns only configured command execution and observation.
- [ ] State that opaque non-zero exit cannot distinguish remote failure from script/API failure; stdout/stderr must carry actionable evidence.
- [ ] State that `service: true` is not appropriate because its zero-exit and restart semantics differ from one finite observation.
- [ ] Explain synchronous and asynchronous control flows, including TOON output and exact-generation follow-up commands.
- [ ] Update README/USAGE/ADVANCED-GUIDE and canonical examples/help only where needed; avoid a GitHub-specific first-class API.
- [ ] Run focused, integration, documentation, and pi-watcher compatibility gates appropriate to touched surfaces.

## Non-goals

- Shipping an external-system sample that requires network access.
- Automatic detection of `git push`.
- Webhooks, provider plugins, structured result protocols, or arbitrary control commands.
- Claiming client await timeout terminates the observed process.
