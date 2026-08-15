# Funzzy One-Hop Output Evidence Contract

> Status: **normative** — defined by TASK-0079. Drives TASK-0080 (typed,
> instance-exact errors), TASK-0081 (paging/bounds), TASK-0082 (exact
> references in correlated snapshots), TASK-0083 (one-hop E2E proof).
> Source research: `.tmp/reports/15-08-26/watcher-output-agent-confusion-session-audit.md`, `docs/AGENT-FEEDBACK-CONTRACT.md` §6, `docs/CLI-V2-CONTRACT.md`.

A real Pi session made eight ineffective `watcher_output` calls before
abandoning the tool and diagnosing by hand. The loop: guessed task identity,
a stale schema decoder, an invalid `tail`+`full` combination, and a
`full` response that exceeded the 64 KiB transport budget. This contract
turns output retrieval into a **one-hop** operation: one observation carries
an exact, copy-safe reference; the agent follows it with **at most one**
retrieval call; every response is below a negotiated budget and self-describes
its bounds.

Goal, restated: one notification/observation followed by at most one
successful retrieval call, without agent-generated identity text.

## §1 Output reference (`outputRef`)

Identity is never reconstructed from prose. The server emits a structured,
copy-safe reference wherever retained output is known to exist:

```json
{
  "outputRef": {
    "instanceToken": "fz-18b4…0123",
    "generation": 42,
    "task": "lint@ci",          // exact machine identity; omitted = whole generation
    "stream": null,             // "stdout" | "stderr" | null (both)
    "mode": "tail",             // "tail" | "page"; never ambiguous
    "tail": 80,                 // safe default when mode = tail
    "maxBytes": 48000           // safe retrieval budget (see §4)
  }
}
```

Rules:

- `instanceToken` is the watcher instance token (`capabilities.instance.token`,
  AGENT-FEEDBACK-CONTRACT §1), matching the existing `cancel` wire param
  (`pi-watcher/src/infra/client.ts`). A reference from a different instance is
  stale by definition and must fail with a typed instance error (§3).
- `generation` is the exact run identity. It is never reused within an
  instance.
- `task` is the exact task identity as the server records it. Display job
  names, tags, and selectors are **metadata, never identity**: the agent
  copies `task` verbatim, it never re-types it from a human summary.
- Whole-generation references omit `task`; one-task references carry it.
  A reference never mixes both (no "all except X" shapes).
- References are frozen at emission time against the same snapshot source
  used by status/await/subscribe/verify, so every surface renders the same
  identity (TASK-0082).
- No reference is emitted before the relevant output is retained, and
  retrieval must still handle the race where output is evicted between
  observation and call (typed eviction error, §3).

### Lifecycle

| Event | Reference validity |
| --- | --- |
| Output retained | Reference valid from first emission until eviction/restart |
| Retention eviction (byte budget) | Typed eviction error on retrieval; reference is not re-emitted after eviction |
| Watcher restart (instance change) | All references from prior instance invalid; typed instance error |
| Generation superseded | Reference still valid while retained; outcome notes supersession |
| Cancellation | Retained partial output retrievable; outcome marks cancelled tasks |
| New generation | Old references unaffected (instance + generation + task scope) |

## §2 Request surface

### Canonical request (from `outputRef`)

```json
{
  "jsonrpc": "2.0",
  "id": "output",
  "method": "output",
  "params": {
    "instanceToken": "fz-18b4…0123",
    "generation": 42,
    "task": "lint@ci",
    "stream": null,
    "mode": "tail",
    "tail": 80,
    "maxBytes": 48000
  }
}
```

- `mode` is one of `tail` (last N lines per stream, bounded by `tail` and
  `maxBytes`) or `page` (deterministic cursor pagination, §5). `tail` and
  `page` are **structurally exclusive**: a request carrying both, or carrying
  paging fields (`cursor`, `pageSize`) with `mode: tail`, is rejected by the
  client/schema **before** the socket call (exit 2, usage error).
- `full` is removed from the preferred agent contract. An unsafe unpaged
  `full` is either rejected or deterministically translated to the first
  bounded page with a continuation cursor — never a response at or above the
  transport budget.
- `instanceToken` is required for advanced retrieval. Legacy clients that omit it
  keep explicit legacy behavior per capability (§4) and never claim exact
  freshness.
- `maxBytes` caps the serialized response; the server clamps to the
  negotiated limit and reports the effective value. Defaults come from the
  reference (`tail` 40–80, `maxBytes` conservative margin below transport).

### Canonical CLI

```sh
fzz ctl output --ref '<outputRef-json>' --format toon   # follow reference exactly
fzz ctl output --instance fz-… --generation 42 --task 'lint@ci' --tail 80 --format toon
fzz ctl output --instance fz-… --generation 42 --page --page-size 2000 --format toon
```

The `--ref` path is the agent-safe one: the shell command is generated from
the structured reference with quoting that survives tags/spaces/quotes
(TASK-0082). Explicit flags are for advanced/human use and are validated by
the same rules.

## §3 Typed RPC errors

Error codes are stable, machine-actionable, and never require parsing message
text. `-32000` (generic server error) is reserved for genuine internal
failures only.

| Code | Name | Meaning | Structured data |
| --- | --- | --- | --- |
| `-32010` | `generation_not_found` | Generation unknown or evicted (pi-watcher maps to `WatcherOutputNotFoundError`) | `{instance, generation, retained: [gen,…], action: "reobserve"}` |
| `-32011` | `task_not_found` | Task unknown in that generation (pi-watcher maps to `WatcherOutputTaskNotFoundError`) | `{instance, generation, task, candidates: [{id, kind}], ambiguous: bool}` |
| `-32012` | `instance_mismatch` | Instance token does not match the active watcher (additive; old clients keep `-32000` legacy path) | `{instance, activeInstance, action: "restart-or-reobserve"}` |
| `-32013` | `invalid_options` | Invalid/mutually-exclusive params or bad cursor | `{field, reason, valid: [...]}` |
| `-32014` | `output_unavailable` | Registry not wired / output disabled | `{feature: "outputRetrieval"}` |
| `-32015` | `internal` | Genuine server failure (maps to legacy `-32000` for old clients) | `{kind}` |

- pi-watcher already expects `-32010` (generation) and `-32011` (task)
  (`pi-watcher/src/infra/client.ts`, `domain/output.ts`). Codes `-32012`..`-32015`
  are additive and must be mirrored in
  `pi-watcher/src/domain/protocol.ts` + fixtures (TASK-0080).
- Legacy servers without the typed codes keep returning `-32000`; pi-watcher
  maps that to the `legacy` profile with freshness `unknown`, never
  `current`.
- Unknown task (`-32012`) carries deterministic canonical candidates.
  A client may resolve **one unambiguous** candidate read-only; multiple or
  zero candidates return the typed error without guessing (§6).

## §4 Capabilities and budget negotiation

`capabilities` gains an explicit output contract section (additive; boolean
`outputRetrieval` alone is insufficient for advanced clients):

```json
{
  "schemaVersion": 2,
  "limits": {
    "outputRetentionBytes": 1048576,
    "maxResponseBytes": 65536,
    "maxEvidenceLines": 40,
    "outputSchemaVersion": 2,
    "outputModes": ["tail", "page"],
    "outputPageSizeMax": 8192,
    "outputMaxBytesEffective": 48000
  }
}
```

- `outputSchemaVersion` drives decoder negotiation. A decoder built for an
  older schema **must fail before the request** with one compatibility error
  naming the upgrade/reload action and `doNotRetry: true` (§6). No parameter
  permutation can turn a schema mismatch into a valid response.
- `maxResponseBytes` is the transport maximum (Pi rejects > 64 KiB today).
  `outputMaxBytesEffective` is the serialized-response guarantee **including
  RPC envelope and encoding margin** — always conservative below transport,
  never equal to it. Every successful output response is below this bound.
- The server reports in every response: `returnedBytes`, `retainedBytes`,
  `observedBytes`, `truncated`, and `nextCursor` when continuation exists
  (§5). These are data, not prose.

## §5 Paging model

Pagination exists so whole-generation retrieval stays under the effective
budget instead of multiplying the limit per task/stream.

- Ordering is deterministic: task identity (server-recorded exact ID), then
  stream (`stdout` before `stderr`), then byte order. A cursor can never skip
  or duplicate bytes.
- A page is scoped to `(instance, generation, task?, stream?)`. Cursors are
  opaque to clients (or validated) and stale/tampered cursors yield
  `-32013 invalid_options` with the valid page range.
- Whole-generation retrieval **shares** the budget: one response may carry
  several tasks' excerpts but never exceeds `outputMaxBytesEffective`; a
  `nextCursor` continues the same logical retrieval.
- Response fields: `nextCursor` (`null` when done), `returnedBytes`,
  `retainedBytes`, `observedBytes`, `truncated`, and per-task/stream
  `content`, `lines`, `truncated`, `retainedBytes`, `observedBytes`.
- Eviction between pages returns `-32011` with the retained range; the client
  re-observes rather than retrying the same page.
- Retention memory stays globally bounded (existing budget, oldest-generation
  eviction); pagination never copies unbounded buffers and never holds the
  registry lock during socket writes.

## §6 Compatibility and retry policy

- One compatibility error per session: `outputSchemaVersion` mismatch →
  `{code: -32020, action: "reload-extension|upgrade-funzzy", doNotRetry: true}`.
- Clients do **not** permute parameters on any typed error. The audit's
  repeated identical failures were caused by exactly this; the contract stops
  it by making every error self-describing.
- Read-only canonicalization is allowed **only** for one unambiguous
  candidate: an unknown task with exactly one candidate may resolve once, and
  the response reports the selected exact ID. Ambiguity schedules no guess.
- `doNotRetry: true` appears on compatibility and instance-mismatch errors;
  the correct action is re-observe/restart, never a parameter variation.
- Old clients (schema 1, boolean `outputRetrieval`) keep additive-compatible
  responses and the `legacy` profile; they never see paging or `outputRef`
  fields, and they never claim exact freshness.

## §7 Security

- Socket boundary is the security boundary: `0600` permissions and socket-path
  ownership (unchanged from AGENT-FEEDBACK-CONTRACT §6). Funzzy does not
  infer secrets and never heuristically redacts command output.
- Documentation and tool guidance state that command output may contain
  secrets; the agent must not echo retrieved evidence into shared logs.
- Defaults are bounded: evidence excerpt ≤ `maxEvidenceLines`, retrieval tail
  ≤ safe default, `maxBytes` clamp below transport. A reference never
  requests more than the negotiated default unless the caller explicitly
  raises it.

## §8 One-hop invariant (proven by TASK-0083)

- One observation (status/await/verify/subscribe notification) that reports a
  failure must contain the exact `outputRef` for the failed task.
- Following that reference must succeed in **one** bounded retrieval call
  (or return one typed, self-describing error that says the exact next
  action).
- No trace may contain shortened task names, repeated parameter permutations,
  or a generic >64 KiB transport failure.

## §9 Out of scope

- Rendering details of TOON/JSON output documents (TASK-0048).
- Subscription transport mechanics (TASK-0050).
- Historical run-duration estimates (TASK-0051..0056).
