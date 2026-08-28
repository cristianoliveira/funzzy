---
id: TASK-0143
title: Prove and document gated watcher registration
status: done
depends_on: [TASK-0142]
priority: normal
tags: [pi-watcher, extension, registration, tdd]
---

# Prove and document gated watcher registration

## Problem

Gated registration spans session start, repeated starts, cwd transitions, and malformed configs — without deterministic proof and documented Pi API limits it will regress silently.

## Context

Verify TASK-0142 against the TASK-0141 contract using the proof routes in `pi-watcher/AGENTS.md` (`make quick`, `make all`; extend `src/e2e.test.ts` only if the loop contract changes — it drives a real composition root against a scripted protocol server).

## Acceptance criteria

- [ ] Deterministic tests prove: absent config (fresh session), present config, both filenames, malformed/invalid config, repeated `session_start`, configured→no-config transition deactivate/reset, and no background behavior in no-config sessions.
- [ ] The Pi unregister limitation (dynamically registered tools cannot be removed; only deactivated) is documented in `pi-watcher/AGENTS.md` alongside the registration route.
- [ ] Public compatibility surfaces for configured projects verified unchanged (`npm pack --dry-run` if package surface touched).
- [ ] `make quick` and `make all` pass; no threshold-sensitive sleeps, no flakiness.
- [ ] Report evidence in `${AGENT_WORKSPACE:-$PWD/.tmp}/reports/` with the verification outcome.

## Verification focus

This is the QA gate for the feature; acceptance criteria mirror the PO report AC #1–5 including the transition caveat.
