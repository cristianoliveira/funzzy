# Funzzy V2 Jobs Configuration Contract

> Status: **normative** — defined by TASK-0075. Drives TASK-0076 (parser/emitter), TASK-0077 (naming boundary), TASK-0078 (migration proof), and the config-discovery chain TASK-0057/0058/0033.
> Source: `.tmp/reports/14-08-26/jobs-config-refactor-plan.md`, `docs/V2-DOCS-ARCHITECTURE.md`, current parser (`src/config.rs`).

## 1. Ubiquitous language

| Term | Definition | Lifetime |
|---|---|---|
| **Job** | Configured workflow unit in `.watch.yaml` (`jobs:` list entry). | Config parse → migration/rewrite |
| **Task** | Execution/outcome of one job within a generation. | Generation plan → task terminal |
| **Command** | Sequential child invocation inside a job. | Spawn → exit |

A **job** is what you configure; a **task** is what runs. The same name appears in both vocabularies on purpose: `jobs:` names the configured unit, and the runtime protocol keeps `tasks` as the execution identity (additive compatibility, §7).

## 2. Preferred root shape

The preferred V2 configuration uses an ordered root `jobs:` list:

```yaml
on:
  change: "src/**"
  concurrency: 2
jobs:
  - name: lint
    parallel: checks
    run: cargo clippy
  - name: test
    parallel: checks
    run: cargo test
  - name: package
    run: cargo build
```

Semantics of `on`, matching, ignore, name/tag, run, cwd/env, init, `parallel`, and hooks/policies are **unchanged** — only the vocabulary at the root changes. Declaration order and contiguous barriers stay semantic: `A@checks, B, C@checks` still means `A -> B -> C` with two separate `checks` occurrences. Jobs do **not** form a GitHub-Actions DAG; there is no `needs:`/`steps:` mapping, no unordered node graph, and no automatic independence.

## 3. Mapping-form rejection

`jobs:` must be an ordered list. A mapping/dict form is rejected with an actionable example that keeps the ordered list shape, because Funzzy order is semantically meaningful:

```yaml
# invalid
jobs:
  lint: { run: cargo clippy }

# valid
jobs:
  - name: lint
    run: cargo clippy
```

Error text must show the ordered-list example and state why order matters (barriers and group occurrences derive from declaration order).

## 4. Compatibility decision

- V2 **emits only `jobs:`** as preferred live syntax (`fzz init` and migration output).
- The existing root list (`- name: ...`) and grouped `on:`/`tasks:` forms remain **accepted** through an explicit compatibility window: they parse and execute exactly as today, but are never presented as preferred syntax in generated output or examples.
- Migration is deterministic: existing forms map 1:1 to the ordered `jobs:` list preserving declaration order, so topology, barriers, and signatures are identical after migration.
- `tasks:` at root inside the grouped form stays accepted; root `jobs:` wins when both are present (see §5 — actually rejected as mixed, §5).

## 5. Locked errors and precedence

| Case | Behavior |
|---|---|
| Both `tasks:` and `jobs:` at root | **Error**, no silent merge — the ambiguity is a config bug, not a preference |
| Duplicate job names | **Error** with the duplicate named |
| Empty `jobs:` | **Error** — a workflow with no units is a config bug |
| Scalar job entry (not a hash) | **Error** with the ordered-list example |
| Mapping-form `jobs:` | **Error** with the ordered-list example (§3) |
| Unknown job property | Same actionable validation as today's task properties |

No silent merge, no partial acceptance: a config is either valid V2 jobs, valid legacy task form, or an explicit error.

## 6. Invariants

- Parallel group names and barrier occurrences remain based on declaration order; renaming `tasks:` to `jobs:` cannot imply a dependency DAG or automatic independence.
- `on.concurrency` remains the scheduler bound; root `jobs:` does **not** introduce an `on.jobs` alias or ambiguity.
- `{{filepath}}`/`{{paths}}` templates, `run` (shell and argv), cwd/env, init, and busy policies are unchanged.

## 7. Protocol and signature effects

- JSON-RPC `tasks` identity remains an **additive compatibility field** for runtime task executions; no protocol rename without a separate revision. The runtime vocabulary is already "task" (contract §1 of AGENT-FEEDBACK-CONTRACT); this contract confirms the boundary and defers any protocol change.
- Duration signatures are unchanged: they encode the resolved plan (name, group/occurrence, commands, cwd/env), not the config vocabulary. Migrating a config to `jobs:` yields the same signature for the same plan.
- Config schema versioning (TASK-0058) declares the `jobs` vocabulary as the current schema; legacy forms decode into the same model.

## 8. Diagnostics

User-facing diagnostics keep the runtime word "task" for executions and add "job" for configuration:

- Config errors: "job 'X' ..." naming the configured unit.
- Runtime failures: "task 'X' ..." naming the execution.
- Migration output (`fzz init --migrate`) prints the deterministic `jobs:` rewrite and reports any legacy form converted.

## 9. Out of scope

- GitHub Actions `needs:`/`steps:`/matrix model — explicitly not adopted.
- Any runtime/protocol rename of "task" — deferred to a separate protocol revision if ever needed.
- Changing `on.concurrency`, matching, barriers, or execution semantics.
