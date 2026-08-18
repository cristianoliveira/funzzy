# V1 → V2 migration table

Every V1 command/flag/config shape, its exact V2 replacement, behavior
change, and exit-code impact. Historical V1 documentation stays valid via
tags (`https://github.com/cristianoliveira/funzzy/tree/v1`); this is the live
migration reference.

## Command/flag migration

| V1 | V2 | Behavior change | Exit-code impact |
| --- | --- | --- | --- |
| `fzz` (zero args) | `fzz` | unchanged (configured watch) | none |
| `fzz <command>` (ad-hoc) | `fzz exec -- PROGRAM ARG...` | argv is preserved end-to-end, never joined/re-parsed | none |
| `fzz --migrate` | `fzz migrate` | explicit subcommand; honors `-c/--config`; atomic rewrite, already-`jobs:` is a byte-identical no-op (exit 0) | none |
| `fzz --non-block` / `-n` | `fzz --on-busy restart` / `--restart` | explicit policy; implies restart with control socket | none |
| `fzz --target <t>` / `-t` | `fzz watch <t>` / `fzz run <t>` | watch-with-no-match is an error; run rejects paths | 1 for no-match/ambiguous |
| `fzz -v` | `fzz -v, --verbose` | unchanged (verbose watch) | none |
| `fzz -V` | `fzz -V, --version` | unchanged | none |
| `fzz --fail-fast` | `fzz -b, --fail-fast` | unchanged | 1 on failure |
| `fzz --restart` | `fzz --restart` | unchanged | none |
| `fzz -l FILE` | `fzz -l, --log-file FILE` | unchanged | none |

## Config shape migration

| V1 shape | V2 shape | Behavior |
| --- | --- | --- |
| root task list (`- name: ...`) | ordered `jobs:` list | accepted compatibility input; rewrite with `fzz migrate`; declaration order/barriers preserved |
| grouped `on:`/`tasks:` | grouped `on:`/`jobs:` | accepted; preferred root is `jobs:`; `fzz migrate` renames the root key |
| `tasks:` + `jobs:` mixed | — | **error** (no silent merge) |
| `--non-block` in examples/scripts | `--on-busy restart` | see flag migration |
| `--target` in examples/scripts | `watch TARGET`/`run TARGET` | see flag migration |

## `fzz migrate` behavior (TASK-0096/0098)

- One responsibility: **transform V1 task vocabulary** in an existing config in place. Never creates from scratch (`fzz init`) or watches. It validates the complete candidate before its atomic replacement; use `fzz check` for explicit validation.
- Honors global `-c, --config <FILE>` (default `.watch.yaml`).
- Inputs: legacy root list → wrapped under `jobs:` (order and comments preserved); grouped `tasks:` → root key renamed; already `jobs:` → byte-identical no-op, exit 0. It does **not** format YAML or relocate `on` fields into `execution` or `hooks`.
- Errors (exit 1, original bytes unchanged): missing file, malformed YAML,
  multiple documents, unsupported root, empty list.
- Write is atomic (same-directory temp + rename); a failed migration never
  leaves a half-written file. Successful output passes `fzz check`.

## Safe migration flow

1. Commit or back up the configuration.
2. Run `fzz migrate` (pass `-c FILE` when it is not `.watch.yaml`).
3. Inspect the ordered `jobs:` rewrite.
4. Move preferred V2 policy fields manually: `on.concurrency`/`on.output` → `execution`, and `on.success`/`on.failure`/`on.close` → `hooks`.
5. Run `fzz check`, then `fzz list`, `fzz run TARGET`, or `fzz watch`.

The manual section edit is deliberately separate from migration. `fzz migrate`
only wraps a V1 root task list or renames root `tasks:` to `jobs:`.

## Diagnostics vocabulary

- Config errors name the **job** (`job 'lint' ...`).
- Runtime failures name the **task**/command (`task 'lint' failed`,
  `Command ... failed`).
- `fzz list` header says "Available jobs".

## New V2 surfaces

- `fzz check` — validate config without a watcher.
- `fzz explain PATH` — matching/ignore + filtered execution topology.
- `fzz config schema|example` — agent-discoverable schema + runnable examples (`example comprehensive|minimal|parallel|agent`).
- `fzz migrate` — explicit, atomic, idempotent legacy-config rewrite.
- `fzz control|ctl ...` — status/list/run/emit/await/cancel/output/capabilities.
- `--events FILE` — NDJSON run event stream.
- `--format toon|json|human` — structured control output.
- `on.debounce`, `on.watch_backend`, `on.respect_gitignore`; `execution.concurrency`; lifecycle `hooks`.
