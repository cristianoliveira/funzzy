# FZZ domain boundaries

## Dependency direction

```text
CLI / configuration / watcher / process / control / stdout / logging adapters
                              ↓
                      domain ports and values
```

Domain planning, matching, generation state, lifecycle arbitration, and
outcomes may depend only on domain values and ports introduced with their first
real consumer. Adapters may depend on domain values and implement those ports.
A domain module must not import a CLI command, YAML/file adapter, filesystem
watcher, process adapter, control-socket transport, stdout/logging adapter, or
watcher runtime.

`tests/domain_boundaries.rs` recursively enumerates every Rust source file
under `src/domain/` and checks it plus the current domain foundations: `rules`,
`plan`, `template`, and `service_lifecycle`. The check reads production source
only, so adapter use in a module's tests does not weaken the rule. Mutation
cases prove direct, aliased, grouped/multiline, and `super` infrastructure
imports fail.

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

## Port introduction rule

No port is published until a domain consumer and its first adapter land in the
same change. This prevents a generic architecture sketch from becoming a public
compatibility surface.

The next consumers define the smallest port at the point of need:

- TASK-0170: configuration validation may need pure input/error values; YAML
  decoding and file discovery stay adapters.
- TASK-0171: execution transitions may need monotonic time, process start/
  observation/stop, and event publication; signals, process groups, sleeping,
  serialization, and output retention stay adapters.
- TASK-0173: typed control/output requests and results belong at the protocol
  edge; control transport itself remains an inbound adapter.

A filesystem watcher registration port is not expected: domain planning needs
normalized path observations, while recursive registration, debounce mechanics,
and reload handoff belong to the watcher runtime.

## Rejected abstractions

- **`ControlTransport` port:** rejected. Control is an inbound adapter that
  turns JSON-RPC/CLI requests into domain commands. Domain rules, plans, and
  transitions do not initiate control requests; adding a transport port would
  invert that responsibility. TASK-0173 will add typed domain requests/results
  at the protocol edge instead.
- **Speculative clock/process/event ports:** rejected for now. The current
  execution model still mixes shell, argv, labels, output capture, readiness,
  process groups, and scheduling in `executor`/`cmd`. Copying a generic subset
  into a public shared type would preserve the coupling and create unused API.
  TASK-0171 owns the transition model and will introduce only the port shapes
  its domain consumer uses.
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
