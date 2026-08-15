---
id: TASK-0087
title: Prove new files and directories trigger watched jobs
status: todo
depends_on: [TASK-0086, TASK-0029, TASK-0043]
priority: high
tags: [integration-tests, watcher, filesystem, create, rename, reliability]
---

# Prove new files and directories trigger watched jobs

## Problem
Unit matching tests do not prove real native notifications, atomic editor saves, nested directory creation, ignored paths, backend parity, and deletion/recreation produce correct generations without duplicates.

## Context

Use event barriers/readiness files and correlated generations, not fixed sleeps or assumptions about notify event order.

## Acceptance criteria

- [ ] Black-box native test starts watcher before path exists, creates matching file, and observes one generation containing exact created path and selected job.
- [ ] Covers file under existing directory, nested missing directories, directory+file burst, atomic temp rename, delete/recreate directory, and create while previous run is busy.
- [ ] Happy/unhappy paths prove matching create runs, unmatched/ignored/gitignored/temp/workspace-escape create does not run, and diagnostics explain decision.
- [ ] Multiple created files in debounce window produce one deterministic changed set; separate windows produce separate correlated generations.
- [ ] Parallel jobs triggered by create preserve barriers/concurrency; explicit sequential comparison changes only effective concurrency.
- [ ] Native and polling backend fixtures assert equivalent selected jobs/paths without asserting identical raw events or tight timing.
- [ ] Watcher/config restart, control await/subscribe/status/output references, cancellation, and stale instance behavior remain exact.
- [ ] No leaked process, socket, temp tree, root log, or watcher thread on pass/failure/timeout.
- [ ] Test fails against implementation that watches only startup-existing concrete files, proving regression sensitivity.
- [ ] README/getting-started and explain examples state that future matching files are covered without restart.

## Notes
