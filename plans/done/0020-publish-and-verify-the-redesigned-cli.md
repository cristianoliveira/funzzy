---
id: TASK-0020
title: Publish and verify the redesigned CLI
status: done
depends_on: [TASK-0016, TASK-0017, TASK-0018, TASK-0019, TASK-0021, TASK-0022, TASK-0023, TASK-0069]
priority: high
tags: [rust, cli, docs, release]
---

# Publish and verify the redesigned CLI

## Problem
A CLI redesign is incomplete until help, usage documentation, both binary aliases, packaging, and migration guidance agree with tested behavior.

## Context

Treat generated help and executable behavior as truth. Coordinate control-socket documentation with `pi-watcher` where command examples change. TASK-0065 through TASK-0069 own the broader V2 documentation revamp; this task is the final CLI publication consistency gate.

## Acceptance criteria

- [ ] README and usage docs lead with configured workflow mode, document `exec` as ad-hoc composition, show control-socket client workflows, and explain verbose diagnostic records.
- [ ] Migration section maps every removed or renamed V1 invocation to V2.
- [ ] `funzzy` and `fzz` expose identical command trees and behavior.
- [ ] Help examples are exercised as smoke tests and no stale `--target`, `--non-block`, or flag convention remains.
- [ ] Shell completion generation is supported or explicitly deferred with rationale.
- [ ] Focused, integration, packaging, and watcher verification gates pass with unchanged worktree fingerprint.
- [ ] Release evidence records intentional compatibility breaks and exit-code behavior.

## Notes

