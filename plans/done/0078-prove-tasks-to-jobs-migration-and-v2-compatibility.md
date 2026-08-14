---
id: TASK-0078
title: Prove tasks to jobs migration and V2 compatibility
status: done
depends_on: [TASK-0076, TASK-0077, TASK-0033]
priority: high
tags: [integration-tests, config, migration, compatibility, jobs, v2]
---

# Prove tasks to jobs migration and V2 compatibility

## Problem
The jobs refactor can silently reorder barriers, change matching, or strand existing .watch.yaml files unless migration and black-box parity are deterministic and reversible before V2 publication.

## Context

Black-box compare parsed topology and observable runs before/after migration rather than only comparing YAML text.

## Acceptance criteria

- [ ] Fixtures cover legacy root list, grouped tasks, preferred jobs, common/nested rules, tags, cwd/env, init, parallel barriers, hooks/policies, and control socket.
- [ ] Migration produces preferred jobs config, preserves comments/order/semantics, and second migration is no-op with exit 0.
- [ ] Before/after `list`, `explain`, local run, watch/init, synthetic emit, control target run, output, and outcomes are semantically equivalent.
- [ ] Parallel high-water/barriers and sequential debugging override remain identical after rename.
- [ ] Duration execution signature/history remains valid for spelling-only migration and invalidates for semantic edits.
- [ ] Mixed/invalid shapes fail deterministically without partial rewrite and backup/recovery is proven.
- [ ] Current accepted compatibility fixtures continue to pass for declared V2 window; emitted docs/examples never teach deprecated tasks form.
- [ ] Config schema/check and agent discovery identify jobs as preferred and give exact migrate command for tasks input.
- [ ] Migration guide states divergence from GitHub Actions mapping/DAG and why ordered list is required.

## Notes

