---
id: TASK-0184
title: Make future-file burst fixture deterministic
status: done
depends_on: []
priority: normal
tags: []
---

# Make future-file burst fixture deterministic

## Problem
The serialized Linux integration suite can lose the first files when a burst creates a directory and immediately writes files before native recursive child-watch registration completes. The test must isolate batching semantics from that OS registration race.

## Context

CI run `34146177258` failed on Ubuntu at `tests/future_files.rs:379` with only `src/file2.rs`, `src/file3.rs`, and `src/file4.rs` observed. The native watcher registers recursive child directories asynchronously; this test should not combine that registration contract with its debounce-burst assertion. Future-directory behavior remains covered by `directory_burst_after_startup_yields_one_canonical_generation`.

## Acceptance criteria

- [x] Create `src/` before starting the watcher.
- [x] Keep the five-file one-window changed-set assertion and later-window assertion unchanged in intent.
- [x] Do not add sleeps or change watcher production behavior.
- [x] Focused target and serialized `future_files` integration suite pass repeatedly.

## Evidence

- Focused target: 20/20 passed with `--test-threads=1`.
- Full serialized `future_files` suite: 5/5 passed with `--test-threads=1`.
- Only `tests/future_files.rs` changes; no production watcher code or sleeps changed.

## Notes

