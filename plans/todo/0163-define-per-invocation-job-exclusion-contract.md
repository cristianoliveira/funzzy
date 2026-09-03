---
id: TASK-0163
title: Define per-invocation job exclusion contract
status: todo
depends_on: []
priority: high
tags: [design, cli, targets, services, worktrees, determinism]
---

# Define per-invocation job exclusion contract

## Problem
Developers often run multiple Funzzy watchers for worktrees of one repository and need the secondary instance to keep normal checks without starting duplicate service jobs. Current target selection is include-only, so avoiding services requires duplicated configuration or artificial tags.

## Desired outcome

A developer can start a secondary worktree watcher that keeps ordinary matching and finite checks while explicitly excluding named targets or every `service: true` job, without maintaining another configuration file.

## Acceptance criteria

- [ ] Define `fzz watch --exclude TARGET` as invocation-only exclusion using the existing target vocabulary (job name, tag, or unambiguous substring), applied after any positive target selection.
- [ ] Define whether `--exclude` is repeatable and specify deterministic behavior for no-match, ambiguous, overlapping, and exclude-everything cases.
- [ ] Define `fzz watch --no-services` as excluding every configured `service: true` job, including legacy and readiness-enabled services.
- [ ] Define how `--exclude` and `--no-services` compose and what users see in startup plans, summaries, control status, target listing, and explain output.
- [ ] Decide explicitly whether local `fzz run` supports the same flags; avoid accidental asymmetric behavior.
- [ ] Preserve configuration, target identity, ordering/group barriers, matching rules, and normal behavior when neither flag is present.
- [ ] Define actionable CLI diagnostics and exit codes for invalid exclusions without silently falling back to all jobs.
- [ ] Record the contract in one canonical document and identify compatibility, help, schema, and pi-watcher impact.

## Non-goals

- Automatic cross-process service discovery, leases, ownership transfer, or failover.
- Repository-wide singleton coordination between independent `fzz` processes.
- New configuration fields for worktree identity.
- Changing service readiness or lifecycle semantics.

## Constraints

Selection must remain deterministic and side-effect free. Excluded services must never spawn, probe readiness, enter the managed pool, or affect generation settlement.

