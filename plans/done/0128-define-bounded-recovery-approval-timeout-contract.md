---
id: TASK-0128
title: Define bounded recovery approval timeout contract
status: done
depends_on: []
priority: high
tags: [design, config, recovery, approval, timeout, determinism]
---

# Define bounded recovery approval timeout contract

## Problem
A recoverable watcher generation can remain non-terminal forever when user leaves interactive approval unanswered, causing agents awaiting watcher completion to stall.

## Context

Extend [`docs/JOB-RECOVERY-CONTRACT.md`](../../docs/JOB-RECOVERY-CONTRACT.md) with one approval-only bound:

```yaml
execution:
  recovery_policy: prompt
  recovery_timeout: 60s
```

Proposed default is `60s`. Expiry is default-deny: preserve original failure, emit an explicit timeout reason, and let exact-generation watcher awaits reach terminal failure. This timeout covers only user approval; recovery and verification commands keep existing process/cancellation behavior.

## Acceptance criteria
- [ ] Define `execution.recovery_timeout` as positive duration using existing duration syntax (`ms`, `s`, `m`), defaulting to `60s`.
- [ ] Define timeout start/end precisely: begins when approval is requested and ends when valid decision arrives, generation is cancelled/superseded, or budget expires.
- [ ] Define expiry as non-affirmative `approval timeout`; no recovery command starts and original job failure is preserved.
- [ ] State that late input is ignored and cannot authorize current/later generation or consume next prompt's approval.
- [ ] Keep timeout frozen with generation config revision; reload affects later generations only.
- [ ] Keep scope approval-only; command runtime timeout and remote approval remain out of scope.
- [ ] Record parser/schema/catalog/init-template, semantic revision hash, diagnostics/events, and pi-watcher compatibility impact.

## Notes

Prefer configuration-only surface for smallest change. A CLI override can be added later from evidence; unlike `recovery_policy: skip`, agents do not need it to guarantee bounded completion.

