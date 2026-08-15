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
| root task list (`- name: ...`) | ordered `jobs:` list | accepted compatibility input; migrate with `fzz init --migrate`; declaration order/barriers preserved |
| grouped `on:`/`tasks:` | grouped `on:`/`jobs:` | accepted; preferred root is `jobs:` |
| `tasks:` + `jobs:` mixed | — | **error** (no silent merge) |
| `--non-block` in examples/scripts | `--on-busy restart` | see flag migration |
| `--target` in examples/scripts | `watch TARGET`/`run TARGET` | see flag migration |

## Diagnostics vocabulary

- Config errors name the **job** (`job 'lint' ...`).
- Runtime failures name the **task**/command (`task 'lint' failed`,
  `Command ... failed`).
- `fzz list` header says "Available jobs".

## New V2 surfaces

- `fzz check` — validate config without a watcher.
- `fzz explain PATH` — matching/ignore + filtered execution topology.
- `fzz config schema|example` — agent-discoverable schema + runnable examples.
- `fzz control|ctl ...` — status/list/run/emit/await/cancel/output/capabilities.
- `--events FILE` — NDJSON run event stream.
- `--format toon|json|human` — structured control output.
- `on.debounce`, `on.watch_backend`, `on.respect_gitignore`, `on.concurrency`.
