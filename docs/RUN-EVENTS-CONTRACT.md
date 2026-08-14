# Funzzy Run Events Contract (NDJSON)

> Status: **normative** — defined by TASK-0039. Drives the `--events FILE`
> flag, the NDJSON sink, and golden stream tests. Reuses the executor event
> model (`src/executor.rs` `Event`) and the correlated identity vocabulary
> from AGENT-FEEDBACK-CONTRACT §1.

## 1. Purpose

Agents and editor integrations need a bounded, machine-readable stream of
task lifecycle and final outcomes without parsing decorated human stdout.
This contract defines that stream: NDJSON written to a dedicated file, never
mixed with human output.

## 2. Destination and framing

- Destination: a file path passed via `--events FILE`. The file is opened
  append-only at command start. Human stdout/stderr and `--log-file` stay
  untouched — the machine stream cannot corrupt them and they cannot corrupt it.
- Framing: **NDJSON** — exactly one JSON object per line, terminated by `\n`.
  Each record is serialized and written atomically (one write per line under
  an exclusive lock), so concurrent events are never byte-interleaved.
- Schema version: every record carries `"schemaVersion": 1`.
- Broken consumer pipe / write failure: the stream logs one warning, then
  disables itself. It never fails the run and never retries a closed pipe.

## 3. Record shape

Every record:

```json
{
  "schemaVersion": 1,
  "event": "<kind>",
  "runId": <generation>,
  "tsMs": <epoch milliseconds>
}
```

Task-level records additionally carry:

```json
{ "task": "<name>", "group": "<occurrence-or-null>" }
```

### Event kinds

| Kind | Emitted by | Extra fields |
|---|---|---|
| `started` | generation start | `trigger`, `batch`, `predecessor`, `changed[]`, `commands[]`, `target`, `effectiveConcurrency`, `concurrencySource` |
| `tick` | task activity | `task`, `group` |
| `task_terminal` | one task reached terminal | `task`, `group`, `state` (`passed`/`failed`/`cancelled`), `durationMs` |
| `finished` | generation terminal | `elapsedMs`, `failures[]`, `supersededBy` |
| `cancelled` | generation cancelled | `supersededBy` |

The `finished` record is the **final** record for a generation and carries the
complete order-independent combined outcome: the `failures[]` list is derived
from the run outcome keyed by task identity (contract §1), never from
completion order. Consumers combine per-task `task_terminal` records with the
final `finished` record; they must not assume any intra-generation ordering.

## 4. Identity and determinism

- Every record carries the run generation (`runId`); task records carry the
  stable task name and group occurrence identity (`group`, e.g. `checks#1`).
- IDs are the same typed identities as the control protocol (AGENT-FEEDBACK
  CONTRACT §1); the stream reuses them without forcing the same transport
  shape.
- Ordering inside a named parallel group is intentionally unspecified;
  consumers key on `task`/`group`, never on stream sequence.

## 5. Bounds

- Output is streamed (flushed per record), never accumulated in memory.
- A consumer that stops reading may cause the writer's pipe/file to fail;
  the sink disables itself with one warning — it never blocks the run.
- Golden tests cover `passed`, `failed`, `cancelled`, and superseded
  generations plus schema compatibility (version constant, required fields).

## 6. Compatibility

- `--events` is additive; without it no stream is opened and behavior is
  byte-identical to today.
- The control protocol's `output` retrieval and correlated snapshots keep
  their own shapes; this stream is a separate, complementary view.
