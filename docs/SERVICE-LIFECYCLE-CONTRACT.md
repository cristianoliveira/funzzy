# Funzzy Service Lifecycle Contract (TASK-0133)

Status: documentation of implemented behavior (TASK-0035 service management,
TASK-0083/0090 generation scheduling and reload reconcile). Two follow-up
changes are specified here and dispatched separately (§6): an actionable
`fzz check` warning and option-catalog help alignment.
Amended after Kely's verification (evidence:
/Users/cristianoliveira/.agents/reports/26-08-26/task-0133-service-lifecycle-contract-verification.md):
local `fzz run` path corrected (§3), service zero-exit terminal semantics
corrected (§2.3), restart-path claim narrowed (§3), reload exception added
to the footgun (§5), warning/help wording refined (§6).

Evidence base (all cited symbols verified in-tree): `executor.rs` (`Run`,
`ActiveTask`, `advance`, `advance_task`, `advance_services`, `cancel`,
`append_plan`, `SERVICE_MAX_RESTARTS`, `SERVICE_RESTART_BACKOFF_MS`),
`workers.rs` (`WorkerCommand::Run` supersede path, `StartServices`,
`ReconcileServices`), `reload_coordinator.rs` (`reconcile_services`),
`watches.rs` (`watch_plan`, `watch_plan_batch`, `target_plan`,
`run_on_init_plan`), `config.rs` (`rule_from_with_common`, `merge_patterns`),
`app.rs` (`check_config`), `option_catalog.rs` (`service` spec).
Reproduction trace: `.tmp/reports/26-08-26/gh-actions-watch-design-a.md`.

## 1. Purpose

A `service: true` job's lifetime is not watcher-owned; it is owned by the
run **generation** that spawned it. Users cannot predict a managed service's
lifetime from the current help text ("started on init, restarted on change,
stopped on exit", `option_catalog.rs`), because "restarted on change"
conflates two different mechanisms: path selection into a *replacement
generation* (re-inclusion) versus an actual watcher-owned restart. This
contract states the real model, derives the init-only service footgun from
it, and records the `fzz check` decision that follows.

## 2. The model: generation ownership

A managed service is a job (`service: true`) that the executor treats as a
background member of one generation:

1. **Membership.** When a service task spawns successfully and is still
   running, `advance` moves it from `run.active` into `run.services`
   (`executor.rs`, TASK-0035). It executes no further generation barrier —
   it is polled, not awaited, by `advance_services`.
2. **Lifetime.** While `run.services` is non-empty the generation never
   reaches `Finished` (`advance` checks `run.services.is_empty()` before
   completing). A generation containing a live service remains running until
   one of exactly three ends:
   - **shutdown** — the watcher exits and reaps everything;
   - **supersession** — a newer generation arrives (§3);
   - **terminal service failure** — the service exhausts its bounded
     restarts and the generation fails (§2.3).
3. **Exit semantics within a generation.** A service's exit is not itself
   a barrier result, but it does carry an outcome:
   - **zero exit = deliberate stop.** The service is removed from the live
     set and records `Passed` (`advance_services`: `TaskState::Passed`,
     `TaskOutcome::Passed`); the generation may then finish successfully
     once no live services remain. It does **not** silently return in a
     later generation.
   - **non-zero exit = unexpected.** Automatic bounded retry:
     `SERVICE_MAX_RESTARTS` (3) attempts with `SERVICE_RESTART_BACKOFF_MS`
     (500ms) backoff; when attempts are exhausted, the service records
     failure and the generation fails with "Service {name} has failed after
     {n} restarts".

## 3. Supersession: restart by re-inclusion

When a new generation arrives while one is active — a filesystem event
batch (`watch_plan_batch`), or a control-socket run selection (`fzz ctl
run TARGET`; only the control path enqueues `WorkerCommand::Run` — local
`fzz run TARGET` executes its own one-shot `RunCommand` process
(`app.rs::Action::Run`) and never supersedes a running watcher) — the
worker cancels and **reaps** the active
generation: `cancel()` shuts down every task in `run.services` and records
each as `Cancelled`. There is no survival of services across a supersede
and no watcher-level service registry.

The replacement generation then runs whatever its **plan selection**
includes. A service continues existing only by being **re-included** in that
plan — i.e. by being selected again from scratch:

- **Path-selected generations** (`watch_plan(path)`): a job joins iff a
  changed path matches its **effective change patterns** (and is not
  ignored, not gitignored, and it is not manual).
- **Run-selected generations** (`target_plan(target)`): a job joins iff its
  name matches the target substring — so `fzz ctl run <service-name>` does
  re-include a named service (and local `fzz run <service-name>` runs it
  once in its own process).
- **Init generations** (`run_on_init_plan`): only at watcher startup, and
  only jobs with `run_on_init: true` (manual jobs never).

There is no other **plan/re-inclusion** restart path beyond watch, target,
and init planning. The service's own **automatic bounded non-zero retry**
(§2.3) is a separate in-place restart mechanism and always preserved. The
only lifecycle that *moves* a live service without a full re-spawn is
**config reload**
(`reload_coordinator.rs::reconcile_services` → `StartServices` /
`ReconcileServices` → `append_plan`, TASK-0090): on a committed valid
reload, removed or signature-changed services are stopped, and added or
signature-changed services are appended into the *active* generation —
by name and signature, regardless of change patterns. Reload never
crosses a supersede.

## 4. Effective change patterns

Whether a service is re-includable by events is decided by config merge, at
parse time (`config.rs::rule_from_with_common`):

```text
effective change = merge_patterns(root on.change, job change)
```

- A service **with** its own `change:` (or with a root `on.change` it
  matches) is re-included into every generation whose triggering path
  matches — it is restarted on those events.
- A service with **empty** effective change patterns is **init-only**: it
  joins only the init generation and explicit run selections naming it.
  Config parsing deliberately allows this shape (a job with
  `run_on_init: true` and no `change` is legal).

## 5. The init-only service footgun

An init-only service (`service: true`, `run_on_init: true`, empty effective
change) is not automatically re-included by unrelated replacement
generations:

1. Watcher starts → init generation owns the service; it runs.
2. Any event (or `ctl run`) that forms a new generation → the active
   generation is cancelled; the service is reaped as `Cancelled`.
3. The replacement plan is path/target-selected; the service matches no
   path → it is absent. Nothing re-includes it: reload reconcile starts
   only **added or signature-changed** services (by name and signature,
   regardless of change patterns — `reconcile_services`); an **unchanged**
   init-only service stays down until a config change touches it, the
   watcher restarts, or an explicit `ctl run` names it.

Observed in the Design A probe: a single config with an init-only mirror
poller service plus a reactor job died after its first poll — the first
mirror write superseded the init generation, and the poller never returned
(`.tmp/reports/26-08-26/gh-actions-watch-design-a.md`, Result 1).

**Do not** "fix" this by giving the service `change: "**"`: it then joins
*every* event plan, and a generation owning a live service never reaches
`Finished` — the normal edit loop never renders results (same probe,
rejected alternative).

**Valid patterns** for a long-lived poller today:

- **Split instances:** run the poller as a dedicated config
  (`fzz watch -c bridge.yaml`) with no other change-triggered jobs in it —
  nothing supersedes it; or run it outside funzzy entirely.
- **Event-driven service:** give the service real change patterns so
  re-inclusion is the desired restart behavior.
- **Watched-scope services** (survive supersede, restart only on reload) is
  a recorded future direction, not today's semantics.

## 6. `fzz check`: actionably warn, do not reject — DECIDED

Decision (lead, recorded): `fzz check` on `service: true` with
`run_on_init: true` and **empty effective change patterns** emits an
**actionable warning**, not a rejection.

Rationale:

1. **The config is legal-but-surprising, not invalid.** It parses, and it
   functions — until the first superseding generation. Rejection would fail
   a configuration that the runtime accepts and that has a legitimate
   standalone-daemon use (the split-instance pattern: a config whose only
   job is an init-only service has nothing to supersede it).
2. **Compatibility surface.** `fzz check` is the same validator the
   templates and existing users run; turning previously-valid configs into
   errors is a breaking change to declared behavior (repo compatibility
   surfaces), and this card's non-goals pin restart/cancel/reload/legacy
   behavior as unchanged.
3. **The failure is contextual, not syntactic.** Whether the footgun fires
   depends on sibling jobs and events, not on the job itself — validation
   cannot know intent; a warning communicates the lifecycle rule exactly
   where the user can act on it.

Warning spec (implementation dispatched to Dave):

- **Site:** `app.rs::check_config`, after `validate_rules` succeeds and
  before the summary `config valid` line. Use the existing `stdout::warn`
  channel (precedent: the missing-paths warning).
- **Condition:** `rule.service() && rule.run_on_init() && rule.watch_patterns().is_empty()`
  — `watch_patterns()` are the **effective** patterns, including those
  inherited from root `on.change` via `merge_patterns`.
- **Message** (actionable, states the model): warns that an init-only
  service is **not automatically re-included by unrelated replacement
generations** (it is reaped on supersession); suggests either adding
  `change:` patterns (re-inclusion restarts it) or isolating it in a
  dedicated config instance (split-instance pattern).
- **Tests:** a `check` unit test asserting the warning fires for the
  init-only shape, and does not fire when effective change patterns exist
  (job-level or inherited from root `on.change`), nor for non-service
  `run_on_init` jobs.

Option-catalog help alignment (also Dave, same dispatch): the `service`
help text (`option_catalog.rs`: "Managed long-running service: started on
init, restarted on change, stopped on exit.") drifts from this contract —
"restarted on change" is only true when effective change patterns match,
and it omits automatic retry. Replace with generation-owned wording that
mentions bounded retry, e.g. "Managed long-running service: owned by the
active generation; restarted by re-inclusion in a replacement generation
and retried up to 3 times on non-zero exit; stopped on supersession or
exit." Add the help-drift test: assert the catalog renders the agreed
wording verbatim so help and this contract cannot diverge silently.

## 7. Invariants preserved

- Restart policy (3 attempts, 500ms backoff), cancellation, reload
  reconcile, and legacy configuration behavior are unchanged.
- `fzz check` remains side-effect-free: the warning adds no filesystem,
  watcher, or socket behavior.
- Non-service `run_on_init` jobs and manual-trigger jobs are untouched
  (manual + service is already rejected at parse time, `config.rs`).
- Nothing in this contract changes generation scheduling, batching, or the
  control protocol.

## 8. Verification

- This document's claims are traceable to the cited symbols; TASK-0133
  verification (Kely) checks each §2–§5 statement against source.
- Follow-up code (warning + help + their tests) lands via TDD under this
  card's dispatch; acceptance for those changes is §6's spec.
