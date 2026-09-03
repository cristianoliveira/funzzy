---
id: TASK-0165
title: Prove and document per-invocation exclusions
status: done
depends_on: [TASK-0164]
priority: high
tags: [integration-tests, docs, cli, targets, services, worktrees, reliability]
---

# Prove and document per-invocation exclusions

## Problem
Users need black-box evidence and clear documentation that exclusions prevent selected jobs and services from spawning without silently changing configuration, matching, or ordinary watcher behavior.

## Desired outcome

Users can confidently reuse one configuration across primary and secondary worktrees, running the secondary watcher with normal finite checks but no duplicate managed servers.

## Acceptance criteria

- [x] Add a spawned-watcher test proving `--no-services` runs matching finite jobs while no legacy or readiness-enabled service process starts.
- [x] Prove `--exclude TARGET` excludes the selected name/tag according to TASK-0163 while all other matching jobs retain order and outcomes.
- [x] Prove positive target selection composes with repeated exclusions and `--no-services` without reintroducing filtered jobs.
- [x] Prove excluded services never appear as active/ready/failed in control status and cannot keep or settle a generation.
- [x] Prove ambiguous, unmatched, and exclude-everything invocations return the approved actionable errors without starting work.
- [x] Prove ordinary watch/run behavior is unchanged when exclusion flags are absent through the existing unchanged-path suites.
- [x] Update README, usage, target documentation, CLI help examples, and migration/release compatibility notes.
- [x] Run focused tests, full configured final gates, and documentation/config drift checks.

## Test constraints

Use process markers and bounded synchronization barriers. Do not use fixed sleeps as evidence that an excluded service did not start; observe the finite generation boundary, then assert the service marker/process is absent.

## Evidence

`tests/watch_exclusions.rs` starts real `fzz` processes with a real Unix control socket. It proves legacy and readiness-enabled services are absent under `--no-services`, tagged exclusions preserve the remaining declaration order, positive selection composes with repeated exclusions and `--no-services`, and invalid selectors fail with exit 2 before socket or job startup. Existing watch/run suites provide the no-filter compatibility coverage.

Focused evidence: `cargo test --test watch_exclusions --features test-integration --no-default-features -- --nocapture` (4 passed). The final configured watcher gate and documentation/config drift checks are recorded in the implementation handoff.

