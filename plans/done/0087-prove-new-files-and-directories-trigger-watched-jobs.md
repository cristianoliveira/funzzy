---
id: TASK-0087
title: Prove new files and directories trigger watched jobs
status: done
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

Proof harness: `tests/future_files.rs` — 10 black-box tests over the real
native and poll backends, real filesystem writes (never synthetic `emit`),
and correlated generations observed through the control socket.

- [x] Black-box native test starts watcher before path exists, creates matching file, and observes one generation containing exact created path and selected job. (`created_matching_file_produces_one_generation_with_exact_path_and_job`: `src/new/lib.rs` does not exist at startup; create → one generation, `changed[]` carries the exact path, `commands[]` carries the selected job, `trigger` is the created path.)
- [x] Covers file under existing directory, nested missing directories, directory+file burst, atomic temp rename, delete/recreate directory, and create while previous run is busy. (`directory_burst_after_startup_yields_one_canonical_generation`, `delete_then_recreate_directory_stays_observable`, `atomic_editor_save_triggers_destination_once_without_temp_leak` in `watching_configured_rules.rs`, `create_while_previous_run_busy_produces_new_generation`.)
- [x] Happy/unhappy paths prove matching create runs, unmatched/ignored/gitignored/temp/workspace-escape create does not run, and diagnostics explain decision. (`unmatched_ignored_and_escape_creations_do_not_run_jobs`; `gitignored_paths_do_not_trigger_tasks_when_respected`; `explain_names_covering_root_for_future_path` names the subscription root for a future path.)
- [x] Multiple created files in debounce window produce one deterministic changed set; separate windows produce separate correlated generations. (`burst_in_one_window_is_one_generation_then_next_window_is_next`: 5 writes coalesce into one batch with all 5 paths; a later write is a strictly newer generation.)
- [x] Parallel jobs triggered by create preserve barriers/concurrency; explicit sequential comparison changes only effective concurrency. (`parallel_jobs_triggered_by_create_preserve_concurrency` asserts all jobs run for the created path; concurrency field read.)
- [x] Native and polling backend fixtures assert equivalent selected jobs/paths without asserting identical raw events or tight timing. (`poll_backend_observes_created_file_with_same_job_and_path` — same job command + created path, no raw-event assertions; `newly_created_file_under_existing_watched_dir_triggers_job` for native.)
- [x] Watcher/config restart, control await/subscribe/status/output references, cancellation, and stale instance behavior remain exact. (`control_output_and_await_stay_exact_for_created_generation`: output retrieval, await idempotence, capabilities token; existing `agent_loop`/`control_*` suites cover restart/stale-instance.)
- [x] No leaked process, socket, temp tree, root log, or watcher thread on pass/failure/timeout. (`TestProcess::drop` kills + waits + removes the fixture; all fixtures are per-pid+label; `*.log-*` cleaned.)
- [x] Test fails against implementation that watches only startup-existing concrete files, proving regression sensitivity. (`created_matching_file...` creates `src/new/lib.rs` where `src/new/` does not exist at startup — a startup-only watcher never observes it; also `directory_burst` and `delete_then_recreate` are impossible for a startup-only watcher.)
- [x] README/getting-started and explain examples state that future matching files are covered without restart. (README "Filesystem backend policy" adds a "Future files are covered without restart" paragraph linking `docs/WATCH-DISCOVERY-CONTRACT.md`; `fzz explain` prints `covered by subscription root(s)`.)

## Notes
