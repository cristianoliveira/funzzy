# Funzzy Agent-Discoverable Configuration Contract

> Status: **normative** — defined by TASK-0057. Drives TASK-0058 (schema +
> examples through `fzz`), coordinates with TASK-0033 (`fzz check`) and
> TASK-0048 (structured output), and the jobs vocabulary from TASK-0075.
> Profile set and init parity: **normative** — TASK-0096 (drives TASK-0097).
> Source: current parser (`src/config.rs`), JOBS-CONFIG-CONTRACT,
> GITIGNORE-CONTRACT, RELEASE-BOUNDARY.

## 1. Purpose

Agents must discover the current `.watch.yaml` shape — fields, constraints,
defaults, compatibility forms, and safe next commands — from the installed
CLI, not from repository or web documentation that can drift. Discovery is
side-effect-free and token-bounded.

## 2. Agent decision loop

```text
fzz config schema [--section S]   # discover structure
fzz config example PROFILE        # get a runnable starting config
(write .watch.yaml)
fzz check [--config PATH]         # semantic validation (TASK-0033)
fzz list | fzz explain PATH       # inspect what would run
fzz run TARGET / fzz watch        # execute
```

Every step is non-interactive, side-effect-free (never starts a watcher,
never opens a socket), and exit-code stable (0 success, 1 invalid/operational,
2 usage).

## 3. Command grammar

```sh
fzz config schema [--section on|job|matching|execution|parallel|control]
fzz config example comprehensive|minimal|parallel|agent
```

- Both commands accept `--format toon|json|human` consistent with TASK-0048;
  `--format json` is the canonical interoperability output.
- Unknown `--section`/PROFILE prints the valid alternatives and exits 2
  (usage) — no socket contact needed.
- Config-free operation: both commands work with no `.watch.yaml` present.
- `comprehensive` (TASK-0096) prints the same bytes `fzz init` writes by
  default (INIT-TEMPLATE-CONTRACT); `config example PROFILE` and
  `fzz init --template PROFILE` are byte-identical for every profile.

## 4. Supported sections and profiles

| Section | Covers |
|---|---|
| `on` | `change`, `ignore`, `socket`, `concurrency`, `debounce`, `watch_backend`, `poll_interval`, `respect_gitignore` |
| `job` | `name`, `run` (shell/argv), `cwd`, `env`, `change`, `ignore`, `run_on_init`, `parallel` |
| `matching` | change/ignore glob semantics, gitignore precedence (GITIGNORE-CONTRACT), `{{filepath}}`/`{{paths}}` |
| `execution` | busy policy, fail-fast, log file, `--sequential`, NDJSON `--events` |
| `parallel` | named contiguous groups, barriers, group occurrences, `on.concurrency` |
| `control` | socket config, `control`/`ctl` subcommands, protocol capabilities |

| Profile | Purpose |
|---|---|
| `comprehensive` | the full commented starter `fzz init` writes by default — small active setup + every supported option documented in comments (INIT-TEMPLATE-CONTRACT) |
| `minimal` | one job, one change pattern — the smallest runnable config |
| `parallel` | two jobs in one group with `on.concurrency` — demonstrates barriers |
| `agent` | control socket + a verify-style job — the agent loop starting point |

`fzz config example` stays stdout-only and side-effect-free: it never gains
file-writing flags, and piping (`> .watch.yaml`) remains the copy mechanism
for agents. Creating a file in the working directory is `fzz init`'s
responsibility alone (CLI-V2-CONTRACT §3a).

## 5. JSON Schema as canonical output

- `fzz config schema --format json` emits valid JSON Schema
  (draft 2020-12) describing the preferred grouped `jobs:` config.
- The schema identifies: version, field type, required/default status,
  enum/range, mutual constraints (e.g. `--wait` requires `--timeout` is CLI;
  config-side: `ignore` vs `change`), deprecation (legacy root list),
  examples, and a note that semantic checks are delegated to `fzz check`.
- Compact text/TOON rendering may be additive (TASK-0048) but never replaces
  valid JSON Schema.
- **Recommended shape**: grouped `jobs:` config (TASK-0075). The legacy root
  task list remains **accepted compatibility input** — documented, never
  emitted by `fzz init`/`example`, and rewritten only by `fzz migrate`.

## 6. Output bounds and determinism

- Schema/example output is bounded and stable: fixed field order, no
  terminal-width dependence, one document on stdout, progress/debug on
  stderr.
- Exit codes: 0 success, 1 config/operational error, 2 usage (unknown
  section/profile/flag).
- Unknown section/profile recovery: the error names the offending value and
  lists valid alternatives.

## 7. Security boundary

Schema and examples never include: environment values, resolved secrets,
filesystem contents, or running watcher state. `env` is described as a
key/value map with values redacted in examples; schema declares the shape,
not the values.

## 8. Single source of truth

One declarative config-spec source (or enforced parity tests) must prevent
drift between schema, examples, CLI help, and parser keys. The optional
generated agent guide/skill is a secondary artifact derived from the same
spec — never an independently maintained truth.

## 9. Out of scope

- Editing or writing `.watch.yaml` (agents write files directly).
- Running watcher state or control-socket introspection from config commands.
- Replacing `fzz check` (TASK-0033) — schema describes shape; `check` validates
  semantics.
