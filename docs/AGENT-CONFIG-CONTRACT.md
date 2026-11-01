# Funzzy Agent-Discoverable Configuration Contract

Agents should ask the installed CLI for configuration shape before they write `.watch.yaml`. `fzz config schema` and `fzz config example` provide bounded, side-effect-free output. `fzz check` validates the result before execution.

## 1. Purpose

The CLI exposes fields, constraints, defaults, compatibility forms, and safe next commands. Repository or web documentation can drift from the installed version.

## 2. Agent decision loop

```text
fzz config schema [--section S]   # discover structure
fzz config example PROFILE        # get a runnable starting config
(write .watch.yaml)
fzz check [--config PATH]         # semantic validation (TASK-0033)
fzz list | fzz explain PATH       # inspect what would run
fzz run TARGET / fzz watch        # execute
```

Discovery and validation (`config schema`, `config example`, `check`, `list`, and `explain`) are non-interactive and do not start a watcher or open a control socket. `run` and `watch` execute work; use them only after validation. Exit codes are stable: 0 for success, 1 for invalid input or an operational error, and 2 for usage.

## 3. Command grammar

```sh
fzz config schema [--section on|execution|hooks|job|matching|parallel|control]
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
| `on` | `change`, `ignore`, `socket`, `debounce`, `watch_backend`, `poll_interval`, `respect_gitignore` |
| `execution` | `concurrency`, default `output` policy |
| `hooks` | generation `success`/`failure` and watcher-session `close` commands |
| `job` | `name`, `run` (shell/argv), `cwd`, `env`, `change`, `ignore`, `run_on_init`, `parallel`, `service`, `output` |
| `matching` | change/ignore glob semantics, gitignore precedence (GITIGNORE-CONTRACT), `{{filepath}}`/`{{paths}}` |
| `parallel` | named contiguous groups, barriers, group occurrences, `execution.concurrency` |
| `control` | socket config, `control`/`ctl` subcommands, protocol capabilities |

| Profile | Purpose |
|---|---|
| `comprehensive` | the full commented starter `fzz init` writes by default — small active setup + every supported option documented in comments (INIT-TEMPLATE-CONTRACT) |
| `minimal` | one job, one change pattern — the smallest runnable config |
| `parallel` | two jobs in one group with `execution.concurrency` — demonstrates barriers |
| `agent` | control socket + a verify-style job — the agent loop starting point |

`fzz config example` writes only to stdout. It has no file-writing flags. Agents can redirect its output with `> .watch.yaml`; `fzz init` is the CLI command that creates the file in the working directory (CLI-V2-CONTRACT §3a).

## 5. JSON Schema as canonical output

- `fzz config schema --format json` emits valid JSON Schema
  (draft 2020-12) describing the preferred grouped `jobs:` config.
- The schema identifies its independent schema version plus field type, required/default status,
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

Keep schema, examples, CLI help, and parser keys in sync through one declarative config spec or enforced parity tests. An optional generated agent guide or skill must derive from that spec, not become a separate source of truth.

## 9. Out of scope

- Editing or writing `.watch.yaml` (agents write files directly).
- Running watcher state or control-socket introspection from config commands.
- Replacing `fzz check` (TASK-0033) — schema describes shape; `check` validates
  semantics.
