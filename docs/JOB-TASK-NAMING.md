# Funzzy Job/Task/Command Naming Boundary

> Status: **normative** — defined by TASK-0077. Drives consistent vocabulary at
> config, CLI, diagnostics, control, and docs boundaries. Extends
> JOBS-CONFIG-CONTRACT §1.

## 1. Glossary

| Term | Definition | Boundary | Lifetime |
|---|---|---|---|
| **Job** | Configured workflow unit in `.watch.yaml` (`jobs:` entry; legacy `tasks:` entry accepted). | config/parser/list/init/`fzz check` | Config parse → migration/rewrite |
| **Task** | Execution/outcome of one job within a generation; stable identity = job name + position, plus group occurrence. | plan/executor/output/control/history | Generation plan → task terminal |
| **Command** | One sequential child-process invocation inside a job/task. | executor/cmd | Spawn → exit |

A **job** is what you configure; a **task** is what runs in one generation;
a **command** is one child process. The same `name` string appears in both
vocabularies deliberately: `jobs:` names the configured unit, and the runtime
protocol keeps `tasks` as the execution identity (additive compatibility,
JOBS-CONFIG-CONTRACT §7).

## 2. Public naming table

| Surface | Use "job" (configured) | Use "task" (runtime) | Notes |
|---|---|---|---|
| Config parsing errors | ✅ `job 'X' ...` | — | invalid job configuration |
| `fzz list` output | ✅ header "Available jobs" | — | lists configured entries |
| `fzz init`/migration | ✅ "jobs:" emitted | — | preferred vocabulary |
| `fzz check` | ✅ "N job(s)" | — | validates config |
| `fzz explain` | plan preview | matched/skipped "task" | shows both vocabularies |
| Runtime failure messages | — | ✅ `task 'X' ...` | failed runtime task/command |
| Run plan / executor | — | ✅ TaskPlan, group occurrence | runtime identity |
| Control JSON-RPC | documented only | ✅ `tasks` keys (unchanged) | additive-compatible, never duplicated for wording |
| Duration history | semantic signature | ✅ task outcomes | signature keyed by plan content |
| Docs / help / examples | `jobs:` | `task` where runtime | one glossary |

## 3. Boundary rules

- **No blind symbol rename.** Public protocol keys (`targets`, `tasks`,
  `matched`, `runId`, …) keep their wire names; wording is additive at
  documentation boundaries only.
- **Plan preserves configured identity.** `TaskPlan.name` is the configured
  job name and `position` is stable; runtime group occurrence (`name#N`) is
  derived at plan build, never parsed from a trigger string.
- **Errors distinguish layers.** Config errors name the job
  (`job 'lint' has no command`); runtime failures name the task
  (`task 'lint' failed`); process errors name the command.
- **Execution signature is semantic.** It encodes the resolved plan
  (name, group/occurrence, commands, cwd/env) — not the YAML vocabulary. A
  `tasks:` → `jobs:` spelling migration alone never invalidates duration
  history; changing job content or topology does.
- **One glossary in docs.** CLI help, schema examples, and migration output
  use `jobs:` for configuration and `task` for runtime executions
  consistently.

## 4. Out of scope

- Any wire-protocol rename of `tasks`/`targets` (deferred to a separate
  protocol revision, JOBS-CONFIG-CONTRACT §7).
- Changing `on.concurrency`, matching, barriers, or execution semantics.
