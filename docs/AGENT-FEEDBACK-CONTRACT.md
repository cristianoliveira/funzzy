# Funzzy Agent Watcher Feedback Contract

> Status: **normative** — defined by TASK-0042. Drives TASK-0043 through TASK-0050.
> Source research: `.tmp/reports/13-04-26/llm-agent-watcher-needs.md`, `docs/CLI-V2-CONTRACT.md`, `docs/PARALLEL-EXECUTION-CONTRACT.md`.

This contract turns the watcher from a log-printing process into a deterministic
state machine and event source an agent can observe, edit against, await, and
diagnose with bounded tool calls. It is the wire-level and CLI-level agreement
that implementation tasks TASK-0043..TASK-0050 build toward. It extends the
existing control protocol additively; nothing in this contract breaks a client
that only speaks the legacy `status` / `targets` / `run` surface.

Core loop served:

```text
observe current verification state -> edit -> await exact fresh generation -> diagnose -> act
```

## §1 Identities and lifetimes

Every correlation field is a typed identity with a defined lifetime. No identity
is derived from timestamps, command strings, or vector positions. All IDs are
unique within one watcher instance; restart changes the instance, and IDs from
different instances are never compared.

| Identity | Definition | Scope | Lifetime | Example |
| --- | --- | --- | --- | --- |
| Watcher instance | One Funzzy watcher process; opaque `token` (`fz-<nanos><pid>`) + `startedAtEpochMs` | Process | Start → exit | `fz-…18b4…0123` |
| Event batch | Maximal set of filesystem/synthetic events coalesced by debounce into one trigger; retains complete normalized changed-path set | Instance | First event → scheduling decision | `batch 7` |
| Generation | One scheduled run-plan execution; monotonic `u64`, never reused or decremented | Instance | Scheduled → terminal or superseded | `42` |
| Task | One workflow task within a generation; stable from run plan through process, output, outcome, and serialization | Generation | Plan build → task terminal | `cli-config` |
| Command | One shell command inside a task; sequential order preserved | Task | Spawn → exit | `(42, cli-config, 2)` |
| Group occurrence | Named concurrency group occurrence; identity `(name, contiguous position)` per PARALLEL-EXECUTION-CONTRACT | Generation | Plan build → group terminal | `unit#1` |

Rules:

- Generation is the external run identity. `run` and `emit` return it so clients
  can await exact work; it is never reused after terminal outcome.
- One normalized event batch maps to **zero or one** generation according to
  scheduling policy (a no-match/ignored batch yields none).
- A generation carries its trigger/batch relation, predecessor, and
  superseded-by identity where applicable.
- Task and group-occurrence IDs stay stable from plan to serialization; verbose
  diagnostics and parallel outcomes consume the same typed identities, not
  duplicate counters.
- Correlation is **evidence of inclusion**, never proof that an edit caused an
  event. Classifier labels: `exact-overlap`, `no-overlap`, `incomplete`,
  `unknown` (pi-watcher `correlation.ts` vocabulary), and a classification never
  upgrades a freshness claim.

## §2 State machine

### Watcher-level states

| State | Meaning | Entered from | Exits to |
| --- | --- | --- | --- |
| `idle` | No pending debounce work and no active generation | terminal / superseded | `batching`, `running` |
| `batching` | Debounce window open; events coalescing, no generation yet | `idle`, `running` | `queued`, back to `idle` (no-op batch) |
| `queued` | Generation scheduled but not yet started (busy policy `wait` or pool full) | `batching` | `running` |
| `running` | At least one task of the current generation executing | `queued` | terminal / superseded |
| `passed` | Current generation terminal, no task failed | `running` | `batching`, `idle` |
| `failed` | Current generation terminal, at least one task failed | `running` | `batching`, `idle` |
| `cancelled` | Current generation terminal by explicit cancel or replacement | `running`, `queued` | `batching`, `idle` |

### Client-observed terminal reasons

A snapshot's `terminalReason` is one of: `passed`, `failed`, `cancelled`,
`superseded`, `timeout`, `disconnected`, `restarted`. The first three describe a
generation's own outcome; the last four describe why an **await** returned
without the awaited generation reaching its own terminal state.

- `superseded`: a newer generation was scheduled under `--on-busy restart`
  before this one finished; the superseded generation's outcome is
  `superseded`, never reported as passed/failed.
- `timeout`: client-imposed bound expired; the watcher performs **no**
  cancellation and the latest snapshot is returned.
- `disconnected`: socket closed or read failed while waiting; the client must
  re-negotiate, never assume continuity.
- `restarted`: instance token changed while waiting (watcher died and was
  replaced); all prior IDs are invalid. A valid config reload hot-swaps
  in-process (TASK-0091) and preserves the instance token — it is not a
  restart.

### Invariants

- A generation transitions `scheduled → queued → running → terminal`; terminal
  states are absorbing.
- `superseded` is a relation plus an outcome, never a state a generation can
  leave.
- A snapshot is internally consistent from one state read: fields can never mix
  generations (one lock guards read + waiter registration + transition).
- Newest-generation-wins under restart policy: a newer scheduled generation
  cancels the active child before replacement work, and stale queued
  generations are discarded before spawn.

## §3 Freshness rule

A result proves the latest observed filesystem state **only when all** hold at
read time:

1. `instance.token` equals the current watcher instance (no restart).
2. No pending debounce work exists after the snapshot (`pendingWork` empty).
3. The snapshot's generation is the latest scheduled generation.
4. That generation is terminal (`passed`/`failed`/`cancelled`).

Freshness classification (`current | stale | unknown`, pi-watcher vocabulary):

- `current`: 1–4 hold. Green here is proof for the latest batch.
- `stale`: a newer generation or pending batch exists after the snapshot; the
  result predates the latest event. Stale green is **not** proof.
- `unknown`: cannot be proven — legacy server without correlation fields,
  instance changed mid-flight, or evidence insufficient. `unknown` never
  implies current.

Rules:

- Freshness is computed at read time and is monotonic: `current` may become
  `stale`, never the reverse.
- Edit correlation classes are reported separately from freshness; an
  `exact-overlap` edit does not upgrade `stale`/`unknown` to `current`.
- The status response always states config fingerprint/version, worktree root,
  watched roots, last observed filesystem sequence, and whether pending debounce
  events exist, so clients can reason about freshness locally.

## §4 Atomic snapshot and await

`await` eliminates the subscribe-after-read race: observe sequence → register
waiter → return happens under one lock, so no transition can be lost between
snapshot read and waiter registration, and no busy-poll is used.

CLI surface (TASK-0044):

```text
fzz control await --after GENERATION --timeout DURATION   # next terminal generation after N
fzz control await --generation GENERATION --timeout DURATION  # exact generation
```

- `--after` and `--generation` are mutually exclusive; both require
  `--timeout` (awaits are always bounded — an unbounded wait is agent-hostile).
- `--after N`: if a terminal generation `> N` already exists, return
  immediately; otherwise block until one reaches terminal. A no-generation-yet
  watcher waits for the first terminal generation after N.
- `--generation N`: return when exact generation N reaches terminal.
- Response is **one consistent snapshot** plus: terminal reason, latest observed
  batch/generation, pending debounce state, and freshness classification.
- Superseded: return with reason `superseded` and the latest snapshot — never
  block forever, never claim passed/failed.
- Timeout: return latest snapshot with reason `timeout`; no cancellation.
- New batch during wait: the returned snapshot reflects it (and the awaited
  generation may classify stale).
- Disconnect/restart: return with explicit reason; never hang.
- `control run/emit --wait` reuse the exact await primitive and return the
  resulting observation in one round trip — no run-then-status-then-fetch
  sequence.

## §5 Run/emit wait, no-match, reconnect

- `run TARGET` schedules and returns `runId` (generation). With `--wait`, it
  follows the exact generation to terminal and returns the observation.
- `emit PATH` routes a synthetic path event through native matching; result
  names matched tasks plus run identity, or an explicit `unmatched`/`ignored`
  outcome with **no scheduled generation**.
- No-match and no-op are explicit, deterministic outcomes (exit `0`, never a
  false green and never a silent empty response); state includes the query
  context so success cannot look like silent failure.
- Timeouts bound both socket read and server wait; a timeout reports the latest
  snapshot and cancels nothing.
- Reconnect: after any disconnect the client re-negotiates `capabilities`; an
  instance token change invalidates every prior ID; `await` against a stale
  instance fails with an explicit `restarted`/`disconnected` reason.

## §6 Output evidence, truncation, redaction, retention

### Retention and bounds

Declared in `capabilities.limits` (additive, `0` = feature absent):

| Limit | Value today | Meaning |
| --- | --- | --- |
| `outputRetentionBytes` | `0` (until TASK-0045) | Global retained task-output cap per watcher; `--full` retrieval can never exceed it |
| `maxResponseBytes` | `65536` | Largest accepted control response; clients fail closed beyond it |
| `maxEvidenceLines` | `40` | Default failure-evidence tail the server emits |

Policy (fixed by TASK-0045):

- Capture is per stream (stdout/stderr) and per task, with a per-stream byte
  bound; capture never duplicates log-file/live output and cannot deadlock child
  pipes.
- Retention is global across generations and tasks: deterministic eviction
  oldest-generation-first; a generation count or TTL bound. Watcher restart
  clears all retained output.
- Truncation is always marked (`truncated: true` + total retained/observed
  size); a failure outcome includes a concise deterministic diagnostic excerpt
  (≤ `maxEvidenceLines`), total size, truncation state, and the exact retrieval
  command (`control output --generation N --task T [--tail 80|--full]`).

### Redaction

- Funzzy does **not** infer secrets: no heuristic redaction of command output.
- Documentation states command output may contain secrets; socket permissions
  (`0600`) and socket-path ownership are the security boundary.
- Structured output never leaks raw stack traces, dependency internals, or
  secrets from Funzzy itself (AXI rule); failure summaries translate dependency
  failures into domain language plus one actionable recovery step.

### Cancellation

- `control cancel --generation N` is compare-and-act on exact identity; a stale
  cancel can never affect a newer generation. Queued/later tasks follow the
  fail-fast/cancel contract; the final outcome identifies cancelled tasks.
- Cancellation owns the entire descendant process tree (TASK-0030); no orphan
  command keeps changing files after the agent moves on.

### Schema negotiation

- One cheap `capabilities` request reports protocol/schema version, watcher
  version, supported methods, optional fields, output formats, limits, and
  features (`atomicAwait`, `subscription`, `correlatedSnapshots`,
  `outputRetrieval`, `pendingWork`, `durationEstimates`, `sequentialOverride`).
- Every feature stays `false` until its implementation task lands; methods list
  only what this server implements. Clients gate await/emit/cancel/output
  retrieval/sequential on negotiated facts, never on package versions.

## §7 Compatibility policy

- Transport stays JSON-RPC 2.0 framed as NDJSON over the Unix socket. Protocol
  extensions are strictly **additive**: existing `status`, `targets`, `run`
  fields and semantics are preserved; new fields appear only in new keys or in
  responses to new methods.
- Unknown method → `-32601 Method not found` with actionable data naming the
  method; parse/invalid-params errors keep standard codes
  (`-32700`, `-32600`, `-32602`). Notifications have no response; requests
  preserve caller ID.
- **Protocol JSON and CLI rendering are distinct contracts.** JSON on the socket
  is the interoperability protocol. TOON is the agent-default CLI rendering
  (AXI) for `control` commands; JSON is the opt-in CLI format for consumers that
  require it. One domain response is encoded at the boundary by whichever
  renderer was selected; progress/debug stays on stderr, structured output on
  stdout, and no terminal-width-dependent output is emitted.
- Older servers without correlation fields keep working through the legacy
  decode path (`status`/`targets`/`run`); negotiation maps them to the
  `legacy` capability profile, and freshness degrades to `unknown` rather than
  being assumed.
- Wire-format changes require matching pi-watcher decoder changes
  (`pi-watcher/src/domain/protocol.ts`, `capabilities.ts`) and golden fixtures
  (`pi-watcher/src/domain/fixtures/*.json`) updated together with any Rust test
  asserting the payload.

### Correlated snapshot schema (normative additive shape)

```text
{
  "instance":     {"token": string, "startedAtEpochMs": number},
  "batch":        {"id": number, "changed": [path, ...]},        // latest batch
  "generation":   {"id": number, "state": string,
                   "trigger": string|null, "predecessor": number|null,
                   "supersededBy": number|null},
  "tasks":        [{"id": string, "groupOccurrence": string|null,
                    "state": string, "exit": number|null,
                    "evidence": {"excerpt": string, "lines": number,
                                 "truncated": bool, "retrieve": string}}],
  "pendingWork":  {"debounceActive": bool, "queuedBatches": number},
  "freshness":    "current" | "stale" | "unknown",
  "terminalReason": "passed" | "failed" | "cancelled" | "superseded"
                    | "timeout" | "disconnected" | "restarted"
}
```

Field names are camelCase; absent optional fields are omitted, never
`null`-padding. The legacy `status` result (generation/state/trigger/commands/
durationMs/failures) is preserved verbatim for old clients.

## §8 Deterministic exit-code matrix

CLI exit codes follow AXI: `0` success/no-op, `1` workflow/operational failure,
`2` usage error. `130` (128+SIGINT) is reserved for Ctrl-C of a local command
line. Structured errors render in the selected format on stdout; stderr carries
only diagnostics.

| Scenario | Command | Exit | Structured outcome |
| --- | --- | --- | --- |
| Success | `control status` / `list` / `capabilities` | 0 | data |
| Clean no-op | `control emit` ignored path; `cancel` already-terminal; `await --after N` already-terminal | 0 | explicit no-op + reason |
| Workflow failure | `control run T --wait` generation `failed`; `await` reason `failed` | 1 | failed snapshot + evidence |
| Superseded | `await`/`run --wait` returns reason `superseded` | 1 | latest snapshot, supersededBy |
| Timeout | `await --timeout 1ns` | 1 | reason `timeout` + latest snapshot |
| Cancel | `control cancel --generation N` accepted; Ctrl-C on local run | 0 / 130 | cancelled outcome / local |
| Disconnect / restart | await while instance changes | 1 | reason `disconnected`/`restarted` |
| Operational failure | socket unavailable, server error, retention-evicted generation | 1 | actionable message + path/alternatives |
| Usage error | unknown target, missing `--timeout`, `--after`+`--generation`, bad flags | 2 | offending input + valid alternatives |

`status` always exits `0` when the observation itself succeeded — a `failed`
state is data, not an exit code.

## §9 Black-box contract matrix

Recorded here before implementation tasks proceed; TASK-0043..0049 add their
focused test matrices on top. Rows marked (impl) land with the named task; rows
marked (now) are already true today.

| # | Surface | Input | Expected | Gate |
| --- | --- | --- | --- | --- |
| 1 | status | idle watcher | `state: idle`, generation 0, exit 0 | now |
| 2 | status | failed generation | failed snapshot, exit 0 | now |
| 3 | targets | running watcher | stable name+commands list | now |
| 4 | capabilities | any | protocolVersion, methods, limits, features all-false | now |
| 5 | run | exact target | `runId` generation, exit 0 | now |
| 6 | run --wait | task fails | terminal failed snapshot, exit 1 | impl (TASK-0044) |
| 7 | emit | matched path | generation + matched tasks, exit 0 | impl (TASK-0022) |
| 8 | emit | ignored path | explicit no-op, no generation, exit 0 | impl (TASK-0022) |
| 9 | await --after | already-terminal gen > N | immediate snapshot, exit 0 | impl (TASK-0044) |
| 10 | await | future completion | blocks then terminal snapshot | impl (TASK-0044) |
| 11 | await | new batch during wait | snapshot reflects it, freshness `stale` | impl (TASK-0044) |
| 12 | await | superseded generation | reason `superseded`, latest snapshot, exit 1 | impl (TASK-0044) |
| 13 | await | watcher restart | reason `restarted`, IDs invalidated, exit 1 | impl (TASK-0044) |
| 14 | await | timeout boundary | reason `timeout`, no cancellation, exit 1 | impl (TASK-0044) |
| 15 | await | `--after`+`--generation`, missing `--timeout` | usage error, exit 2 | impl (TASK-0044) |
| 16 | output | retained generation/task | bounded excerpt + total + truncated flag | impl (TASK-0045) |
| 17 | output | evicted/unknown generation | exit 1, actionable retained range | impl (TASK-0045) |
| 18 | output --full | huge task output | bounded by retention cap, truncation marked | impl (TASK-0045) |
| 19 | cancel | running generation | cancelled outcome, process tree reaped | impl (TASK-0046) |
| 20 | cancel | stale/unknown generation | no-op (terminal) or exit 2 (never existed) | impl (TASK-0046) |
| 21 | cancel | newer generation after request | unaffected; compare-and-act on exact ID | impl (TASK-0046) |
| 22 | structured render | status/await/errors | TOON default, JSON opt-in, clean streams | impl (TASK-0048) |
| 23 | old server | capabilities absent | `legacy` profile, freshness `unknown`, no crash | now |
| 24 | malformed request | bad JSON/params | standard JSON-RPC error codes | now |
| 25 | full loop | observe→edit→await→diagnose→act | ≤ status+await or one run/emit --wait; fixtures within declared budget | impl (TASK-0049) |

### Copyable agent loop (TASK-0049, proven end-to-end)

A successful edit-verify round trip needs at most **one emit + one await**
(or one `run/emit --wait`), and each structured response stays well under the
declared budget. Failures add one `output` retrieval call.

```sh
# 1. Baseline observe (optional; skip when the agent already knows state).
fzz ctl --format toon status

# 2. Edit the file, then trigger and await the exact generation.
fzz ctl emit <path> --format toon          # -> {outcome, matched, runId}
fzz ctl await --generation <runId> --timeout 5m --format toon
#    -> {terminalReason, snapshot:{generation,state,failures}, freshness}

# 3. On failure: retrieve task-attributed evidence, fix, re-await.
fzz ctl output --generation <runId> --tail 80 --format toon
fzz ctl await --generation <newRunId> --timeout 5m --format toon
```

Freshness guarantees (contract §4): the awaited snapshot is exactly the
requested generation and `freshness: current` only when no newer batch
superseded it. `terminalReason` distinguishes `passed`, `failed`,
`cancelled`, `superseded`, `timeout`, `restarted`, and `disconnected` — an
agent never has to guess from stdout. Structured formats emit exactly one
document; progress and debug never mix into stdout.

## §10 Out of scope for this contract

- Subscription transport mechanics (TASK-0050) — the contract keeps
  subscription an optional, capability-gated feature; `await` is
  long-poll-on-demand and remains the baseline guarantee.
- Rendering format details (TOON library selection, TASK-0048).
- Service-task lifecycle beyond cancel semantics (TASK-0035).
