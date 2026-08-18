# Funzzy V2 Configuration Contract

> Status: **normative** — defined by TASK-0116. Drives TASK-0117 (parser and schema), TASK-0118 (V1 migration boundary), TASK-0119 (examples and `fzz init`), TASK-0120 (documentation), and TASK-0121 (end-to-end proof).
>
> This contract intentionally defines the target V2 shape before implementation. Existing parser behavior remains the compatibility baseline until TASK-0117 lands.

## 1. Directional model

A configuration has one direction:

```text
filesystem/control events → on → jobs → execution → hooks
```

- **`on`** receives and processes inputs.
- **`jobs`** is the ordered work declaration.
- **`execution`** controls how selected jobs run.
- **`hooks`** reacts after a generation or watcher session reaches its lifecycle point.

No top-level `version:` property is introduced. The structural shape identifies this V2 form. Funzzy binary version and configuration format are deliberately independent.

```yaml
on:
  change: "src/**"
  socket: .tmp/funzzy/control.sock
  debounce: 500ms

execution:
  concurrency: 2
  output: show-on-failure

hooks:
  success: echo ok
  failure: echo failed
  close: echo closed

jobs:
  - name: test
    run: cargo test
```

`jobs` stays an ordered list. Its declaration order, contiguous `parallel` groups, command behavior, and runtime task identities keep the semantics defined by [JOBS-CONFIG-CONTRACT.md](JOBS-CONFIG-CONTRACT.md).

## 2. Complete V2 property ownership

Every preferred configuration property has exactly one owner. CLI flags are not configuration properties.

| Owner | Properties | Default | Responsibility |
|---|---|---|---|
| root | `on`, `execution`, `hooks`, `jobs` | absent sections use field defaults | compose the configuration; `jobs` is required and non-empty |
| `on` | `change`, `ignore`, `socket`, `debounce`, `watch_backend`, `poll_interval`, `respect_gitignore` | `[]`, `[]`, off, `1s`, `auto`, `500ms`, `false` | event sources, matching inputs, batching, watch backend, and control input |
| `execution` | `concurrency`, `output` | available parallelism, `inherit` | scheduling bound and default job-output policy |
| `hooks` | `success`, `failure`, `close` | none | generation and watcher-session lifecycle reactions |
| `jobs[]` | `name`, `run`, `change`, `ignore`, `run_on_init`, `parallel`, `cwd`, `env`, `service`, `output` | required, required, inherited, inherited, `false`, none, workspace root, inherited, `false`, `execution.output` or `inherit` | ordered units of work and their job-specific overrides |

Job `change` and `ignore` extend their `on` counterparts common-first and deduplicated. Explicit configuration ignore remains stronger than gitignore. Job `output` overrides the execution default; `inherit`, `quiet`, `capture`, and `show-on-failure` retain their existing meanings.

`on.socket` remains under `on`: it accepts control requests, rather than scheduling work or reacting to a result.

## 3. Validation and unknown properties

The parser must reject, rather than ignore:

- an unknown root, `on`, `execution`, `hooks`, or job property;
- a section with the wrong YAML kind;
- invalid enum, duration, boolean, command, or positive-concurrency value;
- missing/empty ordered `jobs`, mapping-form jobs, duplicate job names, and mixed root `tasks` plus `jobs`.

Diagnostics name the exact field path: for example, `execution.concurrency`, `hooks.close`, and `jobs[1].output`. A validation error includes the expected shape/value and a minimal valid example where useful. No field is silently relocated or defaulted after an invalid explicit value.

## 4. Lifecycle and reload semantics

The field owner does not change behavior:

- `on` changes affect input matching, batching, watcher backend, gitignore policy, or control-socket binding.
- `execution.concurrency` affects subsequent scheduling; active work retains its frozen generation configuration. `execution.output` supplies the output policy of jobs that omit `output`.
- `hooks.success` and `hooks.failure` run once for each terminal generation with the established result semantics. `hooks.close` runs at most once for a ready watcher session, has no generation ID, and remains finite with no trigger templates.
- `jobs` changes retain declaration-order, barrier, service, and run-on-init semantics.

A valid hot reload validates a complete candidate and commits it atomically as one runtime revision. An invalid candidate is fatal under the existing reload contract; it must not produce a partial mix of previous and candidate sections. Socket moves retain the bind-new-before-retire-old handoff. Control protocol payloads, pi-watcher negotiation, run events, and runtime task/job semantics are unchanged by this reorganization.

## 5. Compatibility and migration

This shape is a deliberate breaking edit for configurations using the prior grouped placements:

| Prior placement | V2 placement |
|---|---|
| `on.concurrency` | `execution.concurrency` |
| `on.output` | `execution.output` |
| `on.success` | `hooks.success` |
| `on.failure` | `hooks.failure` |
| `on.close` | `hooks.close` |

Those prior placements are **not parser aliases**. Accepting both would create two owners and ambiguous validation, defaults, and reload behavior. Users edit an already-grouped configuration manually when adopting this V2 structure.

This decision does not alter the established `fzz migrate` boundary: migration converts legacy V1 task-list vocabulary to the ordered root `jobs` list only. It neither formats configurations nor relocates `on` properties into `execution` or `hooks`.

Legacy root task-list and grouped `tasks:` acceptance remains governed by JOBS-CONFIG-CONTRACT until its own compatibility decision changes. It must not be conflated with this section reorganization.

## 6. Non-goals and implementation order

This contract adds no new runtime feature, protocol revision, event field, or configuration-version key. It only assigns the existing preferred properties to one V2 owner.

TASK-0117 implements strict parsing and schema from this table. TASK-0118 proves migration stays limited to V1 task-list to `jobs`. TASK-0119 updates generated examples and init. TASK-0120 replaces user-facing V1-layout documentation. TASK-0121 proves the complete configure, migrate, and documentation loop.
