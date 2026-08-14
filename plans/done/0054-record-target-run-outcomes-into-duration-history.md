---
id: TASK-0054
title: Record target run outcomes into duration history
status: done
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

- [x] Fake-clock tests first cover target pass, fail, cancel, supersede, timeout classification, duplicate terminal event, local run, control run, and restart mode.
- [x] Exact target scheduling computes signature from resolved selected plan and attaches target/signature structurally.
- [x] No code parses human trigger strings to recover target or profile identity.
- [x] Successful terminal `Event::Finished.elapsed` records one sample; failure records separate outcome; cancel/supersede do not feed success percentile.
- [x] Run ID to profile association is removed at terminal state and remains bounded during queued/running work.
- [x] Local `fzz run TARGET` and `control run TARGET` use same recording path.
- [x] Filesystem/init/emit runs either remain explicitly unsupported in first slice or use plan-signature profile without contaminating target history.
- [x] Persistence failure emits concise warning but cannot change workflow result or deadlock event delivery.

## Completed

- `src/duration_recorder.rs` (new): `DurationRecorder` projects `Event`s into `DurationHistory` + persists via `DurationStore`. Only target runs (structural `execution_signature`) are associated; Finished(passed)→success sample, Finished(failed)→failure, Finished/Cancelled(superseded)→excluded superseded, Cancelled→excluded cancelled, `note_timeout`→excluded timed-out; duplicate terminals are no-ops; in-flight map bounded (MAX_ASSOCIATIONS eviction); persist is best-effort warning-only. 10 unit tests.
- `RunMetadata` + `Event::Started` gain `target`/`execution_signature` (structural, never parsed from trigger).
- `Worker`: `RunRequest` carries target/signature; new `schedule_target(plan, target)` computes signature from resolved+expanded plan (stores concurrency/fail_fast); consumer attaches via `with_duration_profile`. Trigger label stays `control:<target>` (compat surface).
- `WorkflowRunner`: optional recorder + computes signature from resolved plan when `metadata.target` set. `cli/run.rs` attaches target structurally.
- Composition roots: `app.rs` (local run) and `watch_non_block.rs` (control run) create the recorder with the canonicalized XDG state path; fs/init/emit runs carry no signature and are never recorded.
- Integration: worker-path control-run test records 1 sample; workflow local-run test records 1 sample; fs run not recorded. Full suite: 390 lib + 545 integration tests green.

## Notes

