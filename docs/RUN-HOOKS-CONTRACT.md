# Funzzy Run Hooks Contract

> Status: **normative** — defined by TASK-0040. Generic terminal-outcome hooks
> for notifications and follow-up automation; no platform-specific
> desktop/browser integrations.

## 1. Scope

- **Run-level hooks**: `on.success` and `on.failure` — shell commands run
  once per generation when it reaches a terminal outcome.
- No task/group-level hooks in this revision; the run-level surface covers
  notifications and follow-up without coupling Funzzy to desktops.
- Hooks are **generic and finite**: they are ordinary commands run with the
  same process runner/context as jobs. Composition is the user's choice
  (e.g. a `notify` script), never a built-in desktop integration.

## 2. Semantics

- `on.success` runs exactly once when the generation passes (every task
  passed, no cancellation).
- `on.failure` runs exactly once when the generation fails (any task failed).
- A **superseded** generation (replaced by a newer run) runs **neither**
  hook: it was never a completed outcome, only displaced.
- A **cancelled** generation runs neither hook: cancellation is an explicit
  interruption, not a result.
- Ordering: hooks run after the generation's own tasks reach terminal and
  before the next generation starts.
- Hook failure does **not** change the run outcome or exit code: hooks are
  side effects, and the combined result is computed from the jobs alone.
  Hook failures are surfaced in verbose/structured output for diagnosis.

## 3. Environment and templates

- Hooks expand `{{filepath}}` and `{{paths}}` like jobs (the trigger path and
  batch set of the generation).
- Hooks inherit the workspace root cwd and the process environment of the
  watch session (no synthetic secret injection).

## 4. Recursion and feedback

- A hook may itself change files; those changes are **observable** by the
  watcher and may trigger a new generation — this is intended for loop
  diagnosis and is not suppressed.
- The parser rejects a hook that is itself configured as a watched job
  (a direct feedback loop declaration) as ambiguous.

## 5. Cancellation and reaping

- Hooks use the same process runner, are cancellable, and are reaped when the
  session shuts down or the generation is superseded mid-hook.

## 6. Correlation and output

- Hook events carry the generation identity and appear in verbose
  diagnostics and the NDJSON run-event stream (RUN-EVENTS-CONTRACT) as
  `hook` records, so feedback loops are observable.

## 7. Out of scope

- Desktop/browser notifications (compose via `on.success`/`on.failure`
  commands).
- Task/group-level hooks, DAG-style hook chaining.
- Changing the combined run outcome or exit contract based on hook results.
