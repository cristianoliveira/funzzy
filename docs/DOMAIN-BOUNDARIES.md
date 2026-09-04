# FZZ domain boundaries

## Dependency direction

```text
CLI / configuration / watcher / process / control / stdout / logging adapters
                              ↓
                      domain ports and values
```

Domain planning, matching, generation state, lifecycle arbitration, and
outcomes may depend only on domain values and `domain::ports`. Adapters may
depend on domain values and implement ports. A domain module must not import a
CLI command, YAML/file adapter, filesystem watcher, process adapter,
control-socket transport, stdout/logging adapter, or watcher runtime.

`tests/domain_boundaries.rs` statically checks the current domain candidates:
`domain`, `rules`, `plan`, `template`, and `service_lifecycle`. The check reads
production source only, so adapter use in a module's tests does not weaken the
rule.

## Inventory

| Area | Current owner | Boundary |
| --- | --- | --- |
| Task rules and matching | `rules` | Domain value and matching policy. YAML belongs in `config`. |
| Plan, stages, signatures, outcomes | `plan` | Domain planning and immutable outcome combination. |
| Template expansion | `template` | Pure transformation used by planning; no shell or process ownership. |
| Target selection and explain plan | `watches` | Transitional planning/watcher adapter: it still carries watcher backend and config-policy types. Extract its pure selection values before classifying it as domain. |
| Readiness precedence | `service_lifecycle` | Pure lifecycle arbitration; worker owns process handles. |
| Generation execution | `executor` | Transitional mixed module. TASK-0171 extracts pure transitions and makes it a port consumer. |
| YAML/config/file discovery | `config` | Inbound adapter. TASK-0170 separates decode from domain validation. |
| Filesystem watcher | `watcher`, `watch_loop` | Inbound adapter; emits normalized path observations. |
| Process, output, and runtime worker | `cmd`, `event_stream`, `output`, `workers`, `workflow` | Outbound adapters and orchestration. |
| Control socket and clients | `control`, `control_client`, `cli/control` | Inbound/outbound protocol adapters. |
| CLI and presentation | `app`, `arguments`, `cli`, `stdout`, `logging`, `diagnostics` | Composition and presentation adapters. |

## Ports

`domain::ports` defines the smallest contracts needed by domain behavior:

- `Clock`: monotonic elapsed-time facts, without sleeping or platform clocks.
- `PathObservationSource`: normalized path batches, without watcher setup or
  filesystem event types.
- `ProcessExecutor` and `ProcessHandle`: start, observe, and stop semantics,
  without process groups, signals, child handles, or command-shell types.
- `EventPublisher`: domain-event publication, without serialization, retention,
  stdout, or socket delivery.

Each trait is intentionally free of `Send`, `Sync`, socket, thread, filesystem,
process, and transport details. A runtime adapter adds those constraints where
it needs them.

## Rejected abstractions

- **`ControlTransport` port:** rejected. Control is an inbound adapter that
  turns JSON-RPC/CLI requests into domain commands. Domain rules, plans, and
  transitions do not initiate control requests; adding a transport port would
  invert that responsibility. TASK-0173 will add typed domain requests/results
  at the protocol edge instead.
- **Concrete process-request port type:** deferred. The current command model
  still mixes shell, argv, labels, output capture, and readiness details in
  `executor`/`cmd`. Copying that into a new shared type would preserve the
  coupling. TASK-0171 owns the transition model and will introduce the minimum
  request value then.
- **Filesystem registration port:** rejected. Root registration, recursive
  mode, debounce implementation, and hot-reload handoff are watcher runtime
  concerns. The domain needs normalized observations, not an OS watcher.
- **One broad `Runtime` trait:** rejected. It would collapse unrelated time,
  process, event, and control concerns into a service locator and make fakes
  less precise.

## Migration rule

New domain behavior must first use a value or one of these narrow ports. Do not
move runtime code mechanically into `domain`; make its inputs and outputs
explicit, prove them with fakes, and retain spawned tests for the adapter.
