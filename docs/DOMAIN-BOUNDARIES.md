# FZZ domain boundaries

## Dependency direction

```text
CLI / configuration / watcher / control / stdout / logging adapters
                              ↓
             application ports and runtime adapters
                              ↓
                      domain ports and values
```

Domain planning, matching, generation state, lifecycle arbitration, and
outcomes may depend only on domain values and ports introduced with their first
real consumer. Application ports translate runtime facts and capabilities for
those consumers; runtime adapters implement the ports and own process handles,
signals, capture, and cleanup. A domain module must not import a CLI command,
YAML/file adapter, filesystem watcher, process adapter, control-socket
transport, stdout/logging adapter, or watcher runtime.

`tests/domain_boundaries.rs` recursively enumerates every Rust source file
under `src/domain/` and checks it plus the current domain foundations: `rules`,
`plan`, `template`, and `service_lifecycle`. It uses the maintained `syn`
Rust parser in a dev-only test, so comments, strings, raw byte strings,
lifetimes, nested comments, and conditional items are parsed as Rust rather
than reimplemented lexically. The visitor skips only `#[cfg(test)]` items (not
later production items), rejects imports and fully-qualified
`crate::…`/`super::…` references, and resolves aliases of `crate` and `super`.
Mutation cases prove direct, aliased, grouped/multiline, `super`, compiling
qualified, root-alias, and raw-byte-string behavior.

## Inventory

| Area | Current owner | Boundary |
| --- | --- | --- |
| Task rules and matching | `rules` | Domain value and matching policy. YAML belongs in `config`. |
| Configuration cross-field validation | `config_validation` | Pure validation of neutral inputs; YAML decoding and error presentation remain in `config`. |
| Plan, stages, signatures, outcomes | `plan` | Domain planning and immutable outcome combination. |
| Template expansion | `template` | Pure transformation used by planning; no shell or process ownership. |
| Target selection and explain plan | `watches` | Transitional planning/watcher adapter: it still carries watcher backend and config-policy types. Extract its pure selection values before classifying it as domain. |
| Readiness precedence | `service_lifecycle` | Pure lifecycle arbitration; worker owns process handles. |
| Finite lifecycle transitions | `domain::finite_lifecycle` | Pure start/continue/pass/fail/timeout/cancel decisions; executor translates runtime observations and applies cleanup. |
| Monotonic execution time | `domain::ports::Clock` | Domain-facing time contract; `SystemClock` remains an executor/runtime adapter and `FixedClock`/`ManualClock` provide deterministic fakes. |
| Generation execution | `executor` | Application adapter/orchestrator consuming domain decisions and owning process, output, event, and cleanup mechanics. |
| YAML/config/file discovery | `config` | Inbound adapter. TASK-0170 separates decode from domain validation. |
| Filesystem/path adapters | `watcher`, `watch_loop`, `path_context` | Inbound adapters; emit normalized path observations and resolve filesystem-dependent task cwd containment. |
| Process, output, and runtime worker | `cmd`, `event_stream`, `output`, `workers`, `workflow` | Outbound adapters and orchestration. |
| Control socket and clients | `control`, `control_client`, `cli/control` | Inbound/outbound protocol adapters. Typed internal requests/results (`RetrievalRequest`/`RetrievalError` in `output`, `AwaitParams`/`AwaitSnapshot`, `CancelResult`, `WatcherState`+`FailureEvidence`) keep JSON-RPC/socket shaping at this edge only (TASK-0173). |
| Output retention and retrieval | `output` | Adapter-owned store with typed retrieval contracts: budgets, cursors, eviction, page math, and request validation (`RetrievalRequest`) are pure value rules; capture buffers remain `cmd` process types. |
| CLI and presentation | `app`, `arguments`, `cli`, `stdout`, `logging`, `diagnostics` | Composition and presentation adapters. |

## Port introduction rule

No port is published until a domain consumer and its first adapter land in the
same change. This prevents a generic architecture sketch from becoming a public
compatibility surface.

The next consumers define the smallest port at the point of need:

- TASK-0170: configuration validation uses pure input/error values; YAML
  decoding and file discovery stay adapters.
- TASK-0171: finite/readiness transitions consume semantic observations and
  return decisions. `domain::ports::Clock` is the only extracted execution
  port: lifecycle policy needs monotonic time without runtime types. Process
  start/observation/stop, readiness probes, event publication, signals,
  process groups, sleeping, serialization, and output retention remain
  application/runtime seams because no domain consumer needs their concrete
  contracts.
- TASK-0173: typed control/output requests and results belong at the protocol
  edge; control transport itself remains an inbound adapter. The typed
  request/result values already live beside their owning modules (`output`,
  `awaiting`, `workers`); no new `src/domain` module was introduced because
  no pure-domain consumer exists beyond those adapters, and the boundary
  guard already proves the direction: domain modules import no transport.

A filesystem watcher registration port is not expected: domain planning needs
normalized path observations, while recursive registration, debounce mechanics,
and reload handoff belong to the watcher runtime.

## Rejected abstractions

- **`ControlTransport` port:** rejected. Control is an inbound adapter that
  turns JSON-RPC/CLI requests into domain commands. Domain rules, plans, and
  transitions do not initiate control requests; adding a transport port would
  invert that responsibility. TASK-0173 will add typed domain requests/results
  at the protocol edge instead.
- **Generic process or event ports:** rejected for TASK-0171. `ProcessRunner`
  and `ChildProcess` are consumed by the executor/runtime orchestration and
  expose shell/argv commands, task context, capture handles, exit statuses,
  shutdown outcomes, and Unix signals. `EventSink` publishes the executor's
  runtime-shaped events. Moving these contracts into `domain`, or wrapping
  them without changing their vocabulary, would import infrastructure or
  merely rename it. Pure transitions instead consume semantic observations
  (`ProcessResult`, readiness facts) and return decisions. This preserves the
  application-port boundary without creating a false abstraction.
- **Generic clock port:** not rejected. `domain::ports::Clock` is narrow and has
  a real lifecycle consumer; `SystemClock` is its runtime adapter and existing
  fixed/manual clocks prove deterministic injection.
- **Filesystem registration port:** rejected. Root registration, recursive
  mode, debounce implementation, and hot-reload handoff are watcher runtime
  concerns. The domain needs normalized observations, not an OS watcher.
- **One broad `Runtime` trait:** rejected. It would collapse unrelated time,
  process, event, and control concerns into a service locator and make fakes
  less precise.

## Migration rule

New domain behavior must first use a value or introduce a narrow port with its
first consumer and adapter. Do not move runtime code mechanically into
`domain`; make inputs and outputs explicit, prove them with fakes, and retain
spawned tests for the adapter.
