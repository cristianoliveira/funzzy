---
id: TASK-0054
title: Record target run outcomes into duration history
status: todo
depends_on: [TASK-0052, TASK-0053, TASK-0038, TASK-0043]
priority: high
tags: [rust, executor, workflow, duration, composition, tdd]
---

# Record target run outcomes into duration history

## Problem
Estimator and storage provide no value until exact configured runs carry profile identity and terminal wall durations are recorded without parsing trigger strings.

## Context

Extend structured `RunMetadata` with optional target/profile identity. Composition root combines control-state and duration-recorder event sinks; executor remains persistence-agnostic.

## Acceptance criteria

- [ ] Fake-clock tests first cover target pass, fail, cancel, supersede, timeout classification, duplicate terminal event, local run, control run, and restart mode.
- [ ] Exact target scheduling computes signature from resolved selected plan and attaches target/signature structurally.
- [ ] No code parses human trigger strings to recover target or profile identity.
- [ ] Successful terminal `Event::Finished.elapsed` records one sample; failure records separate outcome; cancel/supersede do not feed success percentile.
- [ ] Run ID to profile association is removed at terminal state and remains bounded during queued/running work.
- [ ] Local `fzz run TARGET` and `control run TARGET` use same recording path.
- [ ] Filesystem/init/emit runs either remain explicitly unsupported in first slice or use plan-signature profile without contaminating target history.
- [ ] Persistence failure emits concise warning but cannot change workflow result or deadlock event delivery.

## Notes

