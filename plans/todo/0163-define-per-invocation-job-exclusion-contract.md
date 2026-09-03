---
id: TASK-0163
title: Define per-invocation job exclusion contract
status: done
depends_on: []
priority: high
tags: [design, cli, targets, services, worktrees, determinism]
---

# Define per-invocation job exclusion contract

## Problem

Developers often run multiple Funzzy watchers for worktrees of one repository and
need the secondary instance to keep normal checks without starting duplicate
service jobs. Current target selection is include-only, so avoiding services
requires duplicated configuration or artificial tags.

## Canonical contract

### 1. Surface and scope

`fzz watch [TARGET] [--exclude TARGET]... [--no-services]` accepts the new
options. `--exclude` is repeatable and preserves argument order for diagnostics.
The options are invocation-only: they are not written to `.watch.yaml`, do not
become part of a reload revision, and do not alter another watcher process.

The zero-argument `fzz` configured-watch alias accepts the same watch options.
`fzz run` deliberately does **not** accept them: it remains the exact finite
single-target operation (`fzz run TARGET`), while exclusion is a watch-session
policy. `fzz ctl` and the Unix control protocol are unchanged; remote callers
cannot mutate a watcher's invocation policy.

### 2. Target vocabulary and resolution

Exclusion selectors use the existing target vocabulary and no new grammar:

* an exact job name selects that job;
* an `@tag` selector selects every job carrying that tag; and
* any other selector must be an unambiguous substring of exactly one job name.

Resolution is against the configured jobs after normal config validation and
before any watch root registration, service spawn, readiness probe, pool
reconciliation, or generation scheduling. Exact name wins when applicable.
Selectors resolve against the original configured set, not the progressively
filtered set, so repeated and overlapping selectors are deterministic.

A missing selector or an ambiguous substring is an actionable CLI error. The
error names the selector and, for ambiguity, lists matching job names in
configuration order. Parsing/semantic invocation errors use exit status 2 and
must never fall back to running all jobs.

A selector matching no jobs is not a no-op. This catches typos and stale target
names. Repeating the same selector, or selectors whose matches overlap, is
accepted and removes each job once.

### 3. Composition and empty plans

Positive selection (`fzz watch TARGET`) is resolved first using existing watch
target semantics. Exclusions are then applied to that selected plan. Repeated
exclusions are set subtraction while declaration order, group boundaries,
barriers, signatures, and path matching of retained jobs remain unchanged.

`--no-services` is equivalent to excluding every configured job with
`service: true`, including legacy services without readiness and readiness-
enabled services. It is applied to the positively selected plan at the same
filtering boundary as `--exclude`; combining it with explicit exclusions is
union-of-exclusions and is idempotent.

An invocation that leaves no runnable jobs is rejected with an actionable error
(exit 2) naming the effective selection and suggesting removal of an exclusion
or a broader target. It must not start a watcher, register a control socket,
spawn/probe a service, or schedule a generation. This includes excluding every
job and selecting a target consisting only of services. A missing/ambiguous
positive target retains its existing diagnostic and exit behavior.

### 4. Service and lifecycle guarantee

A job removed by either option is absent from the effective plan before any
executor lifecycle code runs. In particular an excluded service cannot spawn,
probe readiness, enter the managed pool, affect readiness/active/failed status,
or keep or settle a generation. Service readiness and lifecycle semantics for
retained jobs do not change.

### 5. Observable output

Startup output renders the effective plan and explicitly reports exclusions
(`--exclude` selectors and `--no-services`), in deterministic configuration
order. Diagnostics distinguish configured, selected, and excluded jobs without
printing secrets or changing existing output when neither option is present.

For a filtered watcher, `targets`/startup summaries expose only effective
runnable targets; excluded jobs are not advertised as active targets. Control
`status` reports only retained services and generations. The existing
configuration-oriented `fzz list` and `fzz explain PATH` commands do not accept
watch-only options and therefore continue to describe the unfiltered
configuration; use `fzz watch` startup output to inspect the invocation plan.

### 6. Compatibility and implementation boundary

With neither option present, behavior and output remain byte/behavior
compatible. Configuration schemas, YAML vocabulary, target identity, ordering,
group barriers, matching rules, reload semantics, control JSON-RPC methods, and
Pi-watcher payloads are unchanged. No cross-process ownership, lease, failover,
or singleton coordination is introduced.

Implementation belongs in one shared watch planning/filtering boundary used by
blocking and non-blocking watch modes. CLI parsing/help and focused unit tests
are TASK-0164 work; spawned-watcher and documentation proof are TASK-0165 work.
Both must cover positive selection, repeated/overlapping selectors,
name/tag/substring resolution, no-match, ambiguity, exclude-everything, and
`--no-services` for legacy and readiness-enabled services.

## Acceptance criteria

- [x] Define repeatable `fzz watch --exclude TARGET` using job name, tag, or unambiguous substring, applied after positive selection.
- [x] Define deterministic behavior for no-match, ambiguous, overlapping, repeated, and exclude-everything cases.
- [x] Define `fzz watch --no-services` for every configured `service: true` job, including legacy and readiness-enabled services.
- [x] Define composition and user-visible startup/summary/status/target behavior.
- [x] Decide explicitly that local `fzz run` does not support these watch-only flags.
- [x] Preserve configuration, identity, ordering/group barriers, matching, and no-flag behavior.
- [x] Define actionable diagnostics and exit status 2 without fallback to all jobs.
- [x] Record compatibility, help, schema, and Pi-watcher impact.

## Non-goals

- Automatic cross-process service discovery, leases, ownership transfer, or failover.
- Repository-wide singleton coordination between independent `fzz` processes.
- New configuration fields for worktree identity.
- Changing service readiness or lifecycle semantics.

## Handoff

TASK-0164 may implement the parser and shared filtering boundary from this
contract. TASK-0165 must provide black-box evidence before marking the feature
complete.
