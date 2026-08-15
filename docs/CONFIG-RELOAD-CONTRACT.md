# Funzzy Config Reload Contract

> Status: **normative** — defined by TASK-0088. Drives TASK-0089 (immutable
> runtime config revisions), TASK-0090 (root/policy swap without exit),
> TASK-0091 (control identity across reload), TASK-0092 (reload continuity
> proof). Coordinates with `docs/WATCH-DISCOVERY-CONTRACT.md` (root plan),
> `docs/GITIGNORE-CONTRACT.md` (ignore precedence), `docs/JOBS-CONFIG-CONTRACT.md`
> (settings surface), and `docs/AGENT-CONFIG-CONTRACT.md` (`fzz check`).
> Current behavior anchor: `src/app.rs` `execute_watch_command` (self-SIGTERM
> thread), `src/process_owner.rs` (owned process groups).

Today every real `.watch.yaml` edit sends SIGTERM to the watcher's own PID:
even a valid change destroys process continuity (new instance token, reset
generation IDs, lost retained output). This contract replaces unconditional
self-SIGTERM with a **validate-first branch**: a valid, operationally
preparable candidate hot-reloads in the same process; an invalid or
unpreparable candidate shuts the watcher down gracefully with a nonzero exit
— never a silent stale continuation.

"Kill on invalid" means graceful fatal watcher shutdown **after bounded
validation**, coordinated through process ownership — not an abrupt process
signal that skips cleanup and orphans descendants.

## §1 Validity — four gates and the live point

A candidate is the fully parsed replacement configuration. It becomes **live**
only at the atomic commit (§4). Before commit, four gates run, in order:

1. **Syntactic** — the file parses as YAML (`config.rs` loader).
2. **Schema** — shape constraints hold (`on`/job fields, types, known keys;
   `fzz check` semantics, AGENT-CONFIG-CONTRACT).
3. **Semantic** — values are coherent: positive concurrency, valid durations
   and `watch_backend`, valid globs/patterns (JOBS-CONFIG-CONTRACT),
   root-anchored gitignore enabling, non-conflicting control options.
4. **Operational** — every resource the candidate needs can be prepared
   *before* commit: added watch roots register on the backend, added control
   sockets bind, added services start. A candidate that passes schema but
   cannot prepare is **invalid for reload** and takes the fatal path — it
   must never leave old config running silently.

The exact point a candidate becomes live is the atomic commit (§4): after
commit, events route to the new revision; before commit, nothing observable
changed. There is no window where a half-applied candidate is visible.

## §2 Save handling — debounced, stable-read, atomic-rename aware

- Config saves are **debounced** with the same batch normalization as any
  watched file: transient partial writes (editor truncate-then-write, `>`
  redirect) are not classified before the validation window closes.
- The candidate is read **after** the debounce window settles, and only when
  the file is stable (mtime unchanged across the read); a still-changing file
  re-arms the window instead of being validated mid-write.
- **Atomic editor saves** (temp create/write/rename over destination) and
  **delete/recreate** resolve to the canonical final config path; the
  validation runs against the final content, once (§2 of
  WATCH-DISCOVERY-CONTRACT applies to the config path too).
- The mtime baseline guard (current `app.rs` behavior) stays: stale
  historical events never trigger validation.

## §3 Valid reload preserves continuity

A valid hot reload preserves, without process exit:

- watcher PID/process identity;
- instance token and control service (`capabilities.instance.token`);
- monotonic batch/generation IDs (newer generations strictly follow older);
- retained output (the `OutputRegistry` keeps prior generations' evidence);
- active subscriptions (control `subscribe` streams keep delivering);
- active await connections (they stay open through the reload and their
  observation carries the live lifecycle transition — TASK-0091 AC4).

What *changes* is the effective runtime config: jobs, matching/ignore,
roots, concurrency, debounce, backend, hooks/output policies, sequential
defaults, services, control socket options — classified per §6.

### §3.1 Config lifecycle state source (TASK-0091 AC3)

One shared state source (`ConfigLifecycle`) records config lifecycle
transitions, bounded to a fixed history. The reload thread writes; the
control server and snapshot broker read the same source:

- `configReloading` — a validated candidate is being prepared/committed
  (target revision named);
- `configReloaded` — the commit boundary passed (committed revision named);
- `configInvalid` — terminal; an invalid candidate is shutting the watcher
  down fatally (gate + reason named).

A formatting-only no-op save never transitions the source: the stdout notice
is the only explicit signal and every subsystem stays quiet (no revision
bump, no snapshot churn).

Control surfaces:

- `config` method — the live transition plus the bounded history;
- correlated snapshots carry the live `configLifecycle` transition and
  publish on every transition, so subscriptions receive the revision
  transition without reconnecting;
- `run`/`emit`/`cancel`/`output` expose the frozen config revision of the
  generation they name (additive `revision`/`revisionHash`).

## §4 Atomic commit and the revision boundary

- An immutable runtime snapshot (`ConfigRevision`, TASK-0089) is built fully
  off to the side from the validated candidate. No live object is mutated
  during building.
- Commit is a single pointer swap: after it, later batches use the new
  revision; before it, everything used the old one.
- **Active generations freeze the old revision.** A run already executing
  keeps its revision's plan/outcome/output to terminal state; a valid reload
  never retroactively mutates it. Busy/cancellation policy (restart) applies
  only when the configured policy explicitly replaces an active run — a
  config save alone does not kill finite tasks (TASK-0090 AC).
- **Ordering at the commit boundary is deterministic and observable**:
  batches carry the revision they were routed under; a batch accepted before
  commit uses the old revision, a batch accepted after uses the new one.
  Duplicate events seen by overlapping old/new roots normalize once with one
  revision identity (no double generation at the boundary).
- A **no-op save** (formatting/comment-only, identical effective config)
  reports no-op and does not increment the revision or churn subsystems.

## §5 Invalid candidate — graceful fatal shutdown

An invalid candidate (any of §1's four gates failing) must:

1. emit a **terminal config error** naming the gate and the reason;
2. publish the terminal `configInvalid` lifecycle transition (TASK-0091 AC8)
   when a control surface exists — subscribers observe the terminal event
   before the socket closes (best effort: the process exits right after);
3. **cancel and reap** every owned child/service through
   `process_owner::shutdown_all` (the same coordinated path as Ctrl-C) —
   no SIGKILL shortcut, no panic, no self-SIGTERM;
4. **close resources** (control sockets, retained-output handles, service
   sockets);
5. **exit nonzero** (distinct from clean exit; documented code).

Constraints:

- No orphan descendants: process ownership is the only shutdown path.
- The old config is never left running silently: either the candidate is live
  (valid) or the watcher is gone with a visible terminal error (invalid).
- Bounded validation: the fatal decision comes after the debounce window and
  gate checks, never after an unbounded hang; there is no
  restart-required middle state that keeps stale behavior without telling
  the user.

## §6 Per-setting reload strategy classification

Every schema-valid setting is classified. Each is either **swap** (safe
in-process replacement at commit), **prepare-gated** (must prepare before
commit; failure takes the invalid fatal path), or **out-of-scope** (rejected
as invalid for reload — never a silent no-op):

| Setting | Strategy | Notes |
|---|---|---|
| jobs/topology (add/remove/rename) | swap | new plan at commit; active runs keep old revision (§4) |
| change/ignore globs, gitignore | swap | GITIGNORE-CONTRACT precedence preserved; matcher rebuilt pre-batch |
| watch roots | prepare-gated | added roots register before commit (TASK-0090); missing → fatal |
| concurrency / debounce | swap | applies to generations routed after commit |
| `watch_backend` / `poll_interval` | prepare-gated | backend swap without event-loss gap |
| hooks / output policies | swap | terminal hooks read at run boundary |
| sequential defaults | swap | TASK-0073 semantics for post-commit runs |
| managed services (`service: true`) | prepare-gated | start before commit, retire after (TASK-0090) |
| control socket path/options | prepare-gated | bind new socket before commit; retire old after |
| anything schema-valid but not listed | out-of-scope → invalid fatal | never undocumented restart-required |

## §7 Config deletion/recreation and editor temp behavior

- **Config deleted** (or renamed away): treated as invalid — the watcher
  cannot run without a config and exits gracefully with a terminal error.
- **Config recreated**: validated as a fresh candidate through the same four
  gates; a valid recreation hot-reloads, an invalid one is fatal.
- **Editor temporary files** (`.watch.yaml.tmp`, `~`, `.swp`) that are not
  the final config path never trigger validation; only the canonical config
  path (and its `yaml`/`yml` fallback) does.

## §8 Out of scope

- pi-watcher extension behavior (its own channel/TASK plan).
- Editing/generating `.gitignore` (GITIGNORE-CONTRACT).
- Crash recovery of a half-written config: the debounce + stable-read (§2)
  is the protection; a config that never stabilizes never triggers reload.
