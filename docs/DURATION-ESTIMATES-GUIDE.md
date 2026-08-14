# Funzzy Duration Estimates — User Guide

> Normative contract: `docs/RUN-DURATION-ESTIMATES-CONTRACT.md`.
> Implemented by TASK-0052 through TASK-0056.

Funzzy learns bounded per-target wall-duration distributions from completed
runs, persists them outside the worktree, and exposes a recommended timeout
through the control socket. This is a **hint, not a deadline guarantee**: it
never changes freshness, never implies a run is stuck, and never overrides an
explicit caller timeout.

## Where history lives

```text
${XDG_STATE_HOME:-~/.local/state}/funzzy/workspaces/<workspace-hash>/run-durations-v1.json
```

- `XDG_STATE_HOME` wins; otherwise `~/.local/state` is used.
- `<workspace-hash>` derives from the canonical workspace root plus the state
  schema version — paths are hashed, never exposed over the protocol.
- The file is **outside the watched worktree**, so persisting history never
  dirties the repo and never triggers a watcher feedback loop.
- Writes are atomic (temp file + fsync + rename), permission `0600`, and use
  the watcher as the single writer (last-rename-wins for any accidental
  concurrent process; no silent JSON corruption).

## What is recorded

Only **exact configured target runs** (`fzz run TARGET` and control-socket
`run TARGET`) record history. Filesystem-triggered, init, and synthetic emit
runs carry no target identity and are never recorded, so they cannot
contaminate target estimates.

| Outcome | Effect |
| --- | --- |
| passed (not superseded) | one success sample |
| failed | separate failure diagnostic (never a success baseline) |
| cancelled / superseded / timed-out | counted separately, excluded from percentiles |

The estimator uses **target wall time** — observed end-to-end duration, never
the sum of parallel task durations — measured with a monotonic clock.

## The estimate

For each target with history, `control targets` and correlated snapshots carry:

```json
"estimate": {
  "typicalMs": 38000,
  "upperMs": 61000,
  "recommendedTimeoutMs": 95000,
  "samples": 12,
  "confidence": "medium",
  "source": "measured"
}
```

| Field | Meaning |
| --- | --- |
| `typicalMs` | median of successful durations |
| `upperMs` | nearest-rank p90 of successful durations |
| `recommendedTimeoutMs` | `clamp(max(floor, 10s, p90*1.5 + 2s), 15m)` |
| `samples` | retained success-sample count (max 20 per signature) |
| `confidence` | `none` (0) · `low` (1–2) · `medium` (3–9) · `high` (10+) |
| `source` | `measured` (history) — `configured` reserved for future hints |

A **stable execution signature** (SHA-256 over the resolved plan topology,
commands, shell-vs-argv boundaries, cwd, declared environment content, jobs,
fail-fast, and schema version) keys each profile. Changing any of those
inputs creates a new profile, so stale fast estimates never survive a
workflow change. Environment **values** are hashed, never persisted or
serialized as readable metadata.

The correlated snapshot freezes the estimate **at run start** for the
generation; history changes mid-run never move the progress fields.

## How agents should use it

1. Discover the surface: `capabilities.features.durationEstimates` must be
   `true` (plus declared `limits.durationEstimateLimits`).
2. For a target, read `targets[].estimate.recommendedTimeoutMs`.
3. When the caller omits a timeout, use the recommendation. An **explicit
   caller timeout always wins** and is never silently extended.
4. Never render remaining-time countdowns; report elapsed + historical upper,
   and label elapsed-over-upper as `slower-than-history`, not "stuck".

Precedence: explicit tool argument → measured recommendation → configured
hint/default → current 120s fallback.

## Privacy

- Only duration samples and safe target/task names are stored — no command
  output, no environment values, no file paths from runs.
- The state file is `0600` and local to the machine; it is never committed or
  shared.
- The protocol never exposes signature inputs, environment values, or the
  state-file path.

## Reset and recovery

- **Reset history:** stop the watcher and delete the state file (or the whole
  `funzzy/workspaces/<workspace-hash>/` directory). Restarting the watcher
  starts with empty history.
- **Corrupt/oversized history:** the watcher quarantines the file
  (`run-durations-v1.json.corrupt`), emits one warning, and recovers to empty
  history. Startup and control remain fully usable.
- **Wrong schema version:** same quarantine + empty recovery; the old file is
  preserved for inspection.

## Limitations

- First slice covers **exact configured target wall time** only. Task-level
  prediction and automatic filesystem-plan estimates are later extensions.
- Estimates are machine-local and performance-dependent; do not share history
  across machines.
- Confidence reflects sample count, not run-to-run variance.
- An estimate never cancels a run, never upgrades freshness, and never
  replaces an explicit timeout.
