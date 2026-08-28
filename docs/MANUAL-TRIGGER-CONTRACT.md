# Funzzy Manual-Trigger Contract (TASK-0135)

Status: accepted design. Implementation: TASK-0136. Proof: TASK-0137.
Amended after Kely's verification (evidence:
/Users/cristianoliveira/.agents/reports/26-08-26/task-0135-manual-trigger-contract-verification.md):
control-socket watch exception, `Rules::validate` exception, corrected legacy
rejection sites, `params.sequential` acknowledgment.

## 1. Purpose

Jobs whose commands are meant to run only when a user explicitly asks
(`fzz run TARGET` locally, `fzz ctl run TARGET` on a running watcher) currently
cannot say so. Every job inherits root `on.change` (merged in
`config.rs::rule_from_with_common` via `merge_patterns`), so a job with no own
`change` still matches filesystem events; disabling that today requires
inventing an impossible glob or disabling init globally. This contract defines
the explicit manual-only trigger mode.

Target use case: integration-agnostic command observation (TASK-0137) — a
blocking script must start only after explicit request.

## 2. Shape

```yaml
jobs:
  - name: await-remote
    trigger: manual
    run: ./scripts/await-remote.sh
```

- Property: `trigger`, owner `Job` (`option_catalog::Owner::Job`), values
  `["manual"]` — a closed enum. Reserved for future trigger modes; new values
  require their own contract. Unknown values are actionable errors (catalog
  `values` semantics), never best-effort guesses.
- Preferred V2 `jobs:` only. Declaring `trigger` under legacy `tasks:` (list or
  grouped) is rejected with the same pattern as `recovery`: "supported only in
  preferred V2 jobs". Legacy forms are unchanged in every other respect (§9).
- `trigger` absent = change-triggered job; every existing parse, merge, and
  hash path behaves byte-identically (§9).

## 3. Trigger semantics

`trigger: manual` means exactly: the job is scheduled **only** by an explicit
run-selection command — local `fzz run TARGET` (app.rs →
`Watches::run_target_plan`) or control `fzz ctl run TARGET` (control.rs `run`
→ `run_target` → `target_plan`).

It never means: provider polling, webhook intake, arbitrary control-socket
command execution, or any event-source model. The control `run` method takes a
target name only; commands are always the configured ones (§8).

A manual job:

1. **Does not inherit root `on.change`/`on.ignore`.** Root scope applies to
   change-triggered jobs; a manual job has an empty effective watch/ignore
   surface regardless of the `on` section. Presence of `on` is not an error.
2. **Never matches filesystem events.** Excluded from watch roots
   (`Watches::watch_plan` never selects it; a changed path cannot reach it).
3. **Never runs at watcher initialization.** `run_on_init` selection excludes
   manual jobs unconditionally (also enforced statically, §4).
4. **Never runs via `emit`.** A synthetic path cannot select it (same
   `watch_plan` exclusion); unmatched/ignored semantics are unchanged.
5. **Is inert under `fzz watch`, with one control exception.** A watch whose
   target selection contains only manual jobs is a usage error ("nothing to
   watch"), not a silent idle watcher — **unless the control socket is
   enabled** (`on.socket`): a control-only watcher that exists to serve
   `fzz ctl run` is valid and stays up. Mixed selections watch only their
   change-triggered members.

## 4. Validation (reject ambiguity, never invent precedence)

Parse-time, in `rule_from_with_common` (V2 jobs only). The legacy-V2-only
rejection mirrors `recovery` in **both** legacy parse sites: the root-list
form (`config.rs::rule_from`, where `recovery` is rejected at entry) and the
grouped `on`/`tasks:` form (the `has_tasks` per-task guard inside
`parse_hash_format`'s loop). Rejecting in `rule_from_with_common` alone is
**insufficient** — that function also parses grouped legacy `tasks:` entries.

| Combination | Result |
|---|---|
| `trigger: manual` + own `change` | Error: manual jobs never match filesystem events; remove `change` or `trigger: manual`. |
| `trigger: manual` + own `ignore` | Error: inert on a manual job (nothing to ignore); remove `ignore` or `trigger: manual`. |
| `trigger: manual` + `run_on_init: true` | Error: manual jobs never run at init; remove `run_on_init` or `trigger: manual`. |
| `trigger: manual` + `service: true` | Error: services are started on init and restarted on change (their contract); manual contradicts both. Remove one. |
| `trigger: manual` + root `on.change` present | Valid (root applies to other jobs). |
| `trigger: manual` + `recovery` | Valid. Recovery is a failure-response, not a trigger; approval flow unchanged (JOB-RECOVERY-CONTRACT). |
| `trigger: manual` + `parallel` group | Valid. Group semantics apply when the job is explicitly selected (§5); manual members never enter watch/init plans. |
| `trigger` non-string / unknown value | Error per catalog `values` (e.g. "must be one of: manual"). |
| `trigger` under legacy `tasks:` | Error: supported only in preferred V2 jobs, raised at both legacy parse sites (`rule_from` and the grouped `has_tasks` guard), same shape as `recovery`. |

One **model-level validation exception** is required: `Rules::validate`
rejects any job whose watch patterns are empty and which has no `run_on_init`
("job must contain a `change` and/or `run_on_init` property"). A manual job
has neither — by contract — so `trigger: manual` is an explicit exception
satisfying that invariant: a manual job with no `change`/`run_on_init` is
valid iff `trigger == manual`. The invariant itself is unchanged for every
other job.

All errors are `InvalidConfigError` with actionable hints, matching the
existing strictness style (unknown property, non-boolean `service`).

## 5. Selection semantics

Manual jobs are selected by the **existing** target-selection rules; no
special-casing, no hiding from selectors.

- **Local `fzz run TARGET`** (`run_target_plan`): exact name wins; a non-`@`
  substring must identify exactly one job (ambiguous → error listing matches);
  `@tag` runs every match. Manual jobs participate identically.
- **Control `fzz ctl run TARGET`** (`target_plan`): substring/tag semantics —
  every match runs. Manual jobs participate identically.
- **Known asymmetry (recorded, out of scope here):** the control path applies
  no ambiguity guard the local path has. This predates manual jobs; TASK-0136
  must not change it while implementing manual selection, and any alignment is
  a separate decision.
- Because `@tag` and substring selectors sweep matches by name, a manual job
  with a name sharing a tag/substring runs when that selector is invoked
  explicitly. This is still explicit invocation (the user named the selector);
  users wanting isolation should use unambiguous names.

## 6. Discovery surface

- **`fzz list`** (`rules::available_targets`): manual jobs list their name and
  a line `trigger: manual (explicit run only)` in place of any `change`
  patterns (which they cannot have). `run_on_init`/`recovery` lines as today.
- **`fzz explain PATH`** (`watches.explain`): explain is path-based and manual
  jobs never match paths; output gains a `manual` section naming manual jobs
  ("never match filesystem events; explicit run only") so the absence of a
  match is explained rather than mysterious.
- **`fzz config schema`**: `trigger` appears in the `job` section
  (`option_catalog` Job specs + JSON Schema `job` def): enum `["manual"]`,
  optional, default absent/none.
- **`fzz config example` / `fzz init` profiles**: `comprehensive` gains one
  manual job (template parity byte-identity preserved per
  AGENT-CONFIG-CONTRACT §4).
- **Help**: `OptionSpec::help` for `trigger` states: "Explicit run only
  (`fzz run TARGET` / `fzz ctl run TARGET`); never matches filesystem events
  or runs at init."

## 7. Revision identity and reload

- **Semantic hash:** trigger mode is semantic. `config_revision::encode_rule`
  encodes it (absent = a distinct canonical value from `manual`), and
  `REVISION_SCHEMA_VERSION` is bumped in the same change — bumping invalidates
  old revision hashes by design (documented behavior of
  `config_revision.rs §25`). Formatting-only rewrites keep equal hashes.
- **Reload / frozen generations:** a manual job's scheduled generation freezes
  its revision exactly like any finite job (`RunMetadata::with_revision`).
  A running manual job keeps running under its frozen revision; reload
  reconcile treats it as finite work (unchanged services/`ReconcileServices`
  logic does not apply; `service` is rejected with manual by §4). New explicit
  runs after a valid reload bind to the new revision; an invalid revision is
  rejected by existing reload gates before any swap. Hot reload therefore can
  never retain stale trigger behavior.

## 8. Security boundary

The control surface stays name-selected configuration, unchanged by this
feature:

- Control clients may select only **configured target names** (`run` params are
  a target string; transport validation lives in `control.rs`). The sole
  current optional run parameter is `params.sequential` (TASK-0073: run the
  exact generation with effective concurrency one); `trigger: manual` adds
  no parameter surface beyond it.
- Clients can never supply arbitrary commands, arguments, env, or secrets
  (non-goals, §10); the manual trigger does not add any parameter surface.
- Running a manual job over the socket executes exactly the configured
  commands under the socket's existing permission boundary
  (FLAG_CONTROL_SOCKET).

## 9. Zero behavior change and compatibility

- Every configuration without `trigger:` parses, merges, selects, and hashes
  identically. The only observable delta is the revision-hash invalidation
  from the schema-version bump (§7), which is semantic-versioned and affects
  nothing but hash continuity across the upgrade.
- Root `on.change` merge semantics for non-manual jobs are untouched
  (tasks extend shared rules; dedupe and ordering unchanged).
- Legacy task-list and grouped `tasks:` compatibility unchanged (§2, §4).
- Declared compatibility surfaces (binaries, flags, env vars, command path
  templates, socket methods `status`/`targets`/`run`, feature-gated test
  behavior) unchanged; `targets` responses gain nothing new — manual jobs were
  already listed as targets.

## 10. Non-goals

- Generic opt-out/replace semantics for change-triggered jobs (`!change` etc.).
- Per-invocation arguments, environment injection, or secrets over the socket.
- Execution timeouts (TASK-0138–0140).
- Provider adapters, webhook intake, or structured script results (manual is
  not an event-source plugin model; `trigger` values grow only via contract).
- Changes to `service: true` lifecycle.
- Aligning local vs control ambiguity semantics (recorded in §5).
- Changes to `emit` beyond inherited exclusion.

## 11. Test impact (for TASK-0136)

- config parsing: valid manual job; each rejected combo in §4 (message +
  hint); legacy `tasks:` rejection at BOTH parse sites (root-list `rule_from`
  and grouped `has_tasks` guard — same coverage as `recovery`); unknown value
  rejection; absent-`trigger` regression (all existing config tests must pass
  unchanged).
- validate exception: manual job with no `change`/`run_on_init` passes
  `Rules::validate`; non-manual empty job still rejected.
- matching: `watch_plan`/roots never select manual jobs (with root `on.change`
  present); `emit` cannot reach them; `init_action`/`run_on_init_plan` and the
  init selection exclude them.
- watch semantics: all-manual selection with `on.socket` is a valid
  control-only watcher; without the socket it is a usage error; mixed
  selections watch only change-triggered members.
- selection: `run_target_plan` and `target_plan` include manual jobs (exact,
  substring, @tag); ambiguity behavior unchanged.
- discovery: `list`/`explain`/`config schema`/`config example`/help include
  the manual surface (schema parity tests per AGENT-CONFIG-CONTRACT).
- revision: trigger mode changes the semantic hash; schema-version bump
  invalidates prior hashes; formatting-only rewrite keeps hash.
- reload: running manual generation survives reload under frozen revision;
  post-reload run binds new revision.
- e2e (tests/): manual job reachable via `fzz run` and `fzz ctl run`; never
  triggered by file changes or watcher start.
