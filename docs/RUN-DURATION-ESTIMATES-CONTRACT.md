# Funzzy Historical Run Duration Estimates Contract

> Status: **normative** — defined by TASK-0051. Drives TASK-0052 through TASK-0056
> (Rust) and pi-watcher TASK-0017 through TASK-0019.
> Source research: `.tmp/reports/13-04-26/historical-run-duration-estimates.md`,
> `.tmp/reports/13-04-26/run-estimate-implementation-blueprint.md`,
> `docs/AGENT-FEEDBACK-CONTRACT.md`, `docs/PARALLEL-EXECUTION-CONTRACT.md`.

Funzzy learns bounded per-target wall-duration distributions from completed
runs, persists them outside the worktree, and exposes a recommended timeout
with sample count and confidence. pi-watcher uses the recommendation when the
caller omits a timeout, and renders it compactly. An estimate is a **hint,
never a deadline guarantee**, never a freshness claim, and never a substitute
for an explicit timeout.

The contract extends the control protocol and config **additively**: nothing
here breaks a legacy server, client, or configuration that does not know the
new fields.

Scope for the first implementation slice is **exact configured target wall
time**, because that is what `watcher_verify` needs. Task-level prediction and
automatic filesystem-plan estimates are later extensions, not part of this
contract's mandatory surface.

## §1 Sample eligibility and duration definition

The estimate input is a **target wall duration**: elapsed wall time of one
exact configured target run, from generation start to terminal outcome,
measured with a monotonic clock. Parallel task durations are **never summed**;
group/barrier topology is part of the run signature, and the observed wall time
already contains concurrency effects. Per-task durations are recorded for
diagnostics only and do not feed the target timeout distribution.

Every completed run falls into exactly one outcome class for the estimator:

| Outcome class | Feeds success-timeout distribution? | Counted separately? | Notes |
| --- | --- | --- | --- |
| passed (not superseded) | **yes** | no | Primary distribution. |
| failed | no | **yes** | Separate failure distribution, diagnostics only. |
| cancelled (explicit) | no | **yes** | Outcome exists; excluded from percentiles. |
| superseded | no | **yes** | Never recorded as passed/failed. |
| timed-out (server/observer bound) | no | **yes** | Bound is not a task outcome; excluded. |
| queued time | no | **yes** | Reported separately from execution duration. |
| debounce time | no | **yes** | Reported separately from execution duration. |

Rules:

- Only a **passed, non-superseded** run may add a success sample.
- A run that fails, cancels, or is superseded never removes existing samples;
  it is counted in its own class so diagnostics can explain history.
- Queued and debounce time are never part of the recorded target wall duration
  that feeds the estimator; a target's `durationMs` in snapshots remains
  execution duration as today (AGENT-FEEDBACK-CONTRACT §1).
- Duration is measured with the same monotonic clock as existing executor
  durations; tests inject a fake clock. No wall-clock timestamps are needed for
  recency — insertion order defines the bounded window.

## §2 Deterministic estimator

Locked formulas (implemented and tested in TASK-0052):

| Field | Formula |
| --- | --- |
| `typicalMs` | median of successful durations (nearest-rank, sorted copy) |
| `upperMs` | nearest-rank p90 of successful durations |
| `recommendedTimeoutMs` | `clamp(max(configured_floor, 10_000, upperMs * 1.5 + 2_000), 15 * 60_000)` |
| `samples` | count of retained success samples |
| `confidence` | `none` (0 samples) · `low` (1–2) · `medium` (3–9) · `high` (10+) |
| `source` | `measured` when samples > 0, else `configured` (see §4) |

- Median/p90 use **nearest-rank** on a sorted copy; ties resolve by the
  standard nearest-rank definition, locked by tests (odd/even counts, exact
  p90 boundary index).
- **Retention window:** last **20** success samples per signature, evicted
  oldest-first by insertion order. The window is bounded and deterministic;
  no clock is consulted for eviction.
- **Configured floor:** a project-declared `timeout_hint` (see §4) acts as the
  floor for `recommendedTimeoutMs`. The measured recommendation may grow above
  the hint; it never shrinks below it in this contract revision.
- **Safety margin:** `upperMs * 1.5 + 2s` gives headroom above the p90.
- **Absolute cap:** 15 minutes (`15 * 60_000` ms) regardless of measured values
  or hint. Server and client enforce the same cap.
- **Overflow behavior:** all arithmetic saturates at `u64::MAX` then falls
  under the cap; a malformed/oversized persisted value is rejected at decode
  (see §5) rather than propagated.

## §3 Zero/insufficient history fallback

| Samples | Behavior |
| --- | --- |
| 0 | `estimate` field omitted from `targets`/snapshot; client uses configured hint or default fallback (see §7). Confidence is `none`; source `configured` when a hint exists, else no estimate at all. |
| 1–2 | Confidence `low`. Recommendation still computed from the samples plus floor. |
| 3–9 | Confidence `medium`. |
| 10+ | Confidence `high`. |

- A **fresh** estimate (first sample) is never withheld: one passed run yields
  a `low`-confidence measured estimate.
- A zero-history target must not error; it simply has no measured estimate.
- Optional configured `timeout_hint` (see §4) is the only config-declared
  timeout value in this revision; it is a floor and a zero-history fallback,
  never an override of an explicit caller timeout.

## §4 Configuration surface (additive)

Optional per-task key, parsed by `src/config.rs` and accepted only where it is
meaningful (finite `run` tasks):

```yaml
tasks:
  - name: "@agent-final"
    run: make verify
    timeout_hint: 2m   # optional; floor + zero-history fallback, ms or human duration
```

Rules:

- `timeout_hint` is parsed with the same duration parsing as CLI `--timeout`
  values; a malformed value is a config error with the offending key named.
- It never replaces an explicit caller `timeoutSeconds`; precedence in §7.
- It is **not** a promise of duration; progress vocabulary (§7) applies.
- The hint is included in the target's persisted signature metadata (hashed),
  so changing it invalidates history (TASK-0053).

## §5 Stable execution signature and persistence

### Signature

A stable `ExecutionSignature` is derived from the run plan (TASK-0052), hashed
with a maintained stable algorithm (SHA-256). Canonical serialized input
includes:

- schema version;
- stage order and serial/parallel barriers (PARALLEL-EXECUTION-CONTRACT §2);
- task names and group occurrence identity;
- shell string or argv boundaries (exact, including argument separation);
- resolved relative cwd;
- declared environment key/value **content** (as hash input);
- concurrency `jobs` and `fail_fast`.

Rules:

- Signature output must be stable across process restarts and map iteration
  order; `DefaultHasher` is forbidden (not a compatibility contract).
- **Secret safety:** environment values are hashed, never persisted or
  serialized as readable metadata. Persisted metadata retains only the
  signature hash and safe target/task names.
- Any of the listed inputs changing (command, argv, cwd, env content, topology,
  jobs, fail-fast, timeout hint) produces a **new** signature profile; old
  samples no longer apply (invalidation via profile split, not deletion).

### Persistence (TASK-0053)

Location (outside the watched workspace):

```text
${XDG_STATE_HOME:-~/.local/state}/funzzy/workspaces/<workspace-hash>/run-durations-v1.json
```

| Property | Locked value |
| --- | --- |
| Workspace identity | stable hash of the canonical workspace root (not the volatile temp dir) |
| Schema version | `1`, strict versioned decode; unknown version → quarantine + empty history |
| File name | `run-durations-v1.json` (version in name; `v1` for this contract) |
| Corruption recovery | unreadable/invalid file renamed/quarantined with a warning, history starts empty; never crashes the watcher |
| Permissions | `0600` (owner read/write only) |
| Writer model | single-writer (the watcher process); no concurrent writer contract; explicit inter-process lock or documented single-writer guarantee |
| Atomicity | temp-file write + fsync + rename where supported; no partial file on crash |
| Size limits | bounded profile count and bounded samples per profile (20); file bounded by the product of those bounds plus metadata; oversized file rejected at decode |
| Recency | insertion order, not wall clock (no clock dependency) |
| Watch safety | never written inside the watched workspace; no feedback loop possible |

State is **local to the machine**: it encodes machine-specific performance and
is never committed or shared.

## §6 Additive protocol fields

### Capabilities

Add to the `capabilities` result (`src/control.rs`), additive feature flag:

```json
"features": {
  "...existing...": "...unchanged...",
  "durationEstimates": true
}
```

`durationEstimates` stays `false` until TASK-0055 lands. Clients gate on the
flag, never on package version (AGENT-FEEDBACK-CONTRACT §6).

### `targets`

`ControlTarget` gains an optional field; omitted when no estimate exists:

```json
{
  "name": "@agent-final",
  "commands": ["make verify"],
  "estimate": {
    "typicalMs": 38000,
    "upperMs": 61000,
    "recommendedTimeoutMs": 95000,
    "samples": 12,
    "confidence": "medium",
    "source": "measured"
  }
}
```

- Estimates are computed **at request time**, not frozen at server start.
- All estimate fields are `u64` ms / small enum strings; negative, non-finite,
  and unsafe-integer values are rejected by decoders.
- `typicalMs <= upperMs <= recommendedTimeoutMs` is an invariant; a payload
  violating it is malformed.

### Correlated snapshot (run start)

The correlated snapshot (AGENT-FEEDBACK-CONTRACT §7) gains an optional
`estimate` object captured **at run start**, so progress fields do not move
during the same generation:

```json
{
  "...existing snapshot fields...": "...unchanged...",
  "estimate": {
    "typicalMs": 38000,
    "upperMs": 61000,
    "recommendedTimeoutMs": 95000,
    "samples": 12,
    "confidence": "medium",
    "source": "measured"
  }
}
```

- Snapshot-at-run-start: the estimate is selected when the generation starts
  and stays fixed for that generation's lifetime.
- Estimate in a snapshot **never** changes freshness classification
  (`current | stale | unknown`) and never implies stuck/remaining time.
- Structured run results carry the same optional estimate shape.

### Legacy fallback

- Old clients ignore unknown additive fields (JSON-RPC, AGENT-FEEDBACK-CONTRACT
  §7): unchanged decoders keep working.
- Old servers without the feature keep the `legacy` capability profile;
  pi-watcher falls back to configured/default timeout and renders no estimate.
- Wire-format changes require matching pi-watcher decoder changes
  (`pi-watcher/src/domain/protocol.ts`, `capabilities.ts`) and golden fixtures
  (`pi-watcher/src/domain/fixtures/*.json`) updated together with Rust tests.

## §7 Timeout selection and progress vocabulary

### Selection precedence (pi-watcher, TASK-0018)

```text
1. explicit timeoutSeconds tool argument
2. measured recommendedTimeoutMs (capability present, valid, in bounds)
3. configured timeout hint / default
4. current 120s fallback
```

- Explicit caller timeout **always wins** and is enforced exactly; the server
  absolute cap applies but never silently extends an explicit value.
- Selection returns a typed `TimeoutSelection { milliseconds, source }` with
  `source ∈ explicit | measured | configured | default` plus chosen estimate
  metadata (samples, confidence) for tool details.
- `watcher_verify` discovers the exact target before choosing a timeout and
  uses the same target identity for the run request.

### Progress vocabulary

- Never emit remaining-time countdowns; elapsed + historical upper only:

```text
RUNNING gen=184 elapsed=44s expected~38s upper=61s timeout=95s
```

- Once elapsed exceeds `upperMs`, report `slower-than-history`, never "stuck".
- An estimate never upgrades freshness, never changes a failure, and never
  causes cancellation; it is observational.

## §8 Golden fixture matrix (written before implementation)

Rust tests and pi-watcher golden fixtures (`.json` files) lock the wire shape
and estimator behavior before TASK-0052..0056 implement it. Each row has a Rust
test and, where the wire shape is involved, a pi-watcher fixture.

### Estimator matrix (TASK-0052, pure, fake clock, no sleeps)

| # | Input samples (ms) | Expected typical/upper/recommended | Confidence |
| --- | --- | --- | --- |
| 1 | [] | no estimate (or configured-only) | none |
| 2 | [40000] | 40000 / 40000 / clamp(10s, 62s) | low |
| 3 | [30000, 50000] | 40000 / 50000 / clamp(77s) | low |
| 4 | [10000,20000,30000] | 20000 / 30000 / clamp(47s) | medium |
| 5 | 20 samples, odd/even boundary | nearest-rank median/p90 | high |
| 6 | outlier 10× upper | p90 not mean; bounded by cap | high |
| 7 | 21st sample | oldest evicted; window stays 20 | high |
| 8 | u64::MAX sample | saturates, then cap | high |
| 9 | floor 2m, samples low | recommendation = max(floor, margin) | low |
| 10 | floor 2m, samples huge | recommendation = min(cap, ...) | high |

### Signature matrix (TASK-0052)

| # | Change | Same signature? |
| --- | --- | --- |
| 1 | map insertion order / task order in hash input | yes (must not alter) |
| 2 | command string | no |
| 3 | argv boundary | no |
| 4 | resolved cwd | no |
| 5 | env content | no |
| 6 | topology/barriers | no |
| 7 | jobs / fail-fast | no |
| 8 | timeout_hint | no |

### Persistence matrix (TASK-0053, temp dirs)

| # | Scenario | Expected |
| --- | --- | --- |
| 1 | missing file | empty history, no error |
| 2 | corrupt JSON | quarantine + warning, empty history |
| 3 | unknown schema version | quarantine + empty history |
| 4 | oversized file | rejected at decode, empty history |
| 5 | atomic write | temp + rename; no partial file observed |
| 6 | permissions | file created `0600` |
| 7 | 21+ samples | bounded at 20, oldest-first eviction |
| 8 | signature change | new profile; old samples untouched |
| 9 | concurrent writer | documented single-writer policy holds |

### Wire matrix (TASK-0055 + pi-watcher TASK-0017/0018/0019)

| # | Scenario | Expected wire behavior |
| --- | --- | --- |
| 1 | `targets` with history | optional `estimate` present, invariants hold |
| 2 | `targets` no history | no `estimate` key (absent, not null) |
| 3 | `capabilities` | `features.durationEstimates` true/false correctly |
| 4 | snapshot at run start | estimate fixed for generation; fields camelCase |
| 5 | malformed estimate (negative/NaN/order) | decoder rejects; client fails closed |
| 6 | oversized recommendedTimeout | capped; never exceeds server absolute bound |
| 7 | legacy server (no feature) | fallback path; `legacy` profile; no regression |
| 8 | explicit timeout | wins exactly; `source: explicit` |
| 9 | measured recommendation | chosen when timeout omitted; `source: measured` |
| 10 | zero history | default/configured; `source: default|configured` |
| 11 | restart persistence | estimate survives watcher restart |
| 12 | workflow change | old recommendation removed until new samples exist |
| 13 | failed/cancelled/superseded | never lower later success timeout selection |

pi-watcher fixtures: `estimate.json` (present), `estimate-malformed.json`,
`estimate-legacy.json` under `pi-watcher/src/domain/fixtures/`, kept in sync
with Rust tests asserting the same payloads (AGENT-FEEDBACK-CONTRACT §7).

## §9 Out of scope for this contract

- Task-level per-task timeout prediction and automatic filesystem-plan
  estimates (explicitly deferred; scope is exact configured target wall time).
- Hot-reload semantics of estimates while a run is in flight (snapshot is
  fixed at run start).
- Sharing history across machines (state is local by design).
- Changing existing `durationMs` semantics in snapshots.
