# Funzzy V2 CLI Contract

> Status: **draft** — defined by TASK-0014. Drives TASK-0015 through TASK-0024.
> Source research: `.tmp/reports/13-04-26/cli-interface-review.md`, `.tmp/reports/13-04-26/similar-cli-inspiration.md`.

This is a V2 redesign. During the active refactor we do **not** add deprecated parser paths. Each removed V1 invocation is listed under [Migration](#migration).

## 1. Command tree

| Command | Default action | Reads stdin? |
| --- | --- | --- |
| `fzz` | Configured watch — all matching tasks | no |
| `fzz watch [TARGET] [options]` | Configured watch, optional target filter | no |
| `fzz run TARGET [options]` | Run selected configured workflow once locally | no |
| `fzz list [options]` | Print configured tasks (name, tags, change patterns) | no |
| `fzz explain PATH` | Print tasks a path would match / ignore (no execution) | no |
| `fzz init [--migrate]` | Create or migrate `.watch.yaml` | no |
| `fzz exec [options] -- PROGRAM [ARG...]` | Ad-hoc watch over stdin-supplied paths | **yes** |
| `fzz control status` | Print running watcher state over Unix socket | no |
| `fzz control list` | Print remote targets from running watcher | no |
| `fzz control run TARGET [--wait] [--timeout DUR]` | Trigger named target; optionally await terminal outcome | no |
| `fzz control emit PATH [--wait] [--timeout DUR]` | Submit synthetic path change through native routing | no |

`fzz` with no arguments is the **hero path** and stays zero-config.

## 2. Target matching

- A `TARGET` is a task name, `@tag`, or substring.
- Matching is substring over name and tags, documented explicitly.
- `watch TARGET` with no match is an **actionable error**, not silent all-tasks.
- Local `run TARGET` gives exact name precedence; `@tag` may select many tasks, while another substring must be unambiguous. It runs full selected workflow and rejects path arguments.
- `control run TARGET` uses watcher target matching over running watcher; unlike local `run`, it requires IPC.
- `list` and `explain` never execute tasks.

## 3. Option ownership

Global options valid on `fzz`, `fzz watch`, `fzz run`, `fzz exec`:

- `-c, --config <FILE>`
- `--on-busy <wait|restart>` (default `wait`); `--restart` is a short alias for `--on-busy restart`
- `-l, --log-file <FILE>`
- `--log-truncate-on-change` (requires `--log-file`)
- `--control-socket <PATH>` (implies `--on-busy restart`)
- `--no-run-on-init`
- `-v, --verbose` (repeatable: `-v`, `-vv`)
- `-V, --version`
- `-h, --help`

Scoped options:

- `init` owns `--migrate`.
- `control` owns `--socket <PATH>`, `--wait`, `--timeout <DUR>`.
- `exec` owns the trailing `-- PROGRAM [ARG...]`.
- `watch` owns the optional positional `TARGET`.

Irrelevant option/subcommand combinations (e.g., `init --wait`, `list --on-busy restart`) **fail explicitly** with a usage error.

## 4. Flag conventions

| Flag | V1 | V2 |
| --- | --- | --- |
| verbose | `-V` | `-v, --verbose` (repeatable) |
| version | `-v` | `-V, --version` |

This is an intentional break from V1. Migration documents it.

## 5. Busy-run policy

Replaces V1 `--non-block`.

- `--on-busy wait` (default): finish the active run before processing the next change.
- `--on-busy restart` / `--restart`: cancel the active child and schedule the newest generation deterministically.
- `--control-socket` implies restart so status/run stay live.
- `FUNZZY_NON_BLOCK=1` maps to `--on-busy restart`. `FUNZZY_BAIL=1` maps to fail-fast.

## 6. Streams, exit codes, precedence

- `--help` / `--version`: stdout, exit `0`.
- Parse errors / invalid combinations: stderr, exit `2`.
- Runtime/config errors: stderr, exit `1`.
- Child non-zero exit in `exec`: surfaced as a run failure; Funzzy reports it without masking.
- Environment precedence: CLI flag > environment variable > `.watch.yaml` > default.

## 7. stdin semantics

- Only `exec` reads stdin, as a newline-separated list of paths/globs.
- `fzz` with no arguments and an empty/non-tty stdin does **not** silently switch to `exec`. Piped input without `exec` is a usage error.
- `FUNZZY_STDIN_TIMEOUT_MS` governs the stdin grace period (default 2000 ms), retained.

## 8. Control socket

- Path resolution: `--socket` > `--control-socket` > `.watch.yaml` `on.socket` > error.
- Wire format unchanged: JSON-RPC 2.0 framed as NDJSON. Existing `status`, `targets`, `run` contracts preserved.
- New `emit` method (TASK-0022): `{"method":"emit","params":{"path":"..."}}` → result names matched tasks and run identity or an explicit `unmatched`/`ignored` outcome with no scheduled generation.
- `control run --wait` and `control emit --wait` track the returned generation to a terminal state (`passed`, `failed`, `cancelled`) or timeout; a superseded generation (`generation > runId`) resolves deterministically.
- Client I/O has bounded connect/read/wait timeouts. Raw command output stays on the watcher's stdout/log file; client output stays compact.

## 9. Verbose / logging

- Levels via repeatable `-v` (summary) and `-vv` (detail) per TASK-0023.
- Stable greppable prefix; each line carries structured context (task, generation, trigger path).
- Covers: matched task per path, ignored skip, generation transition, busy-run decision, cancellation, template expansion.
- Mirrored to `--log-file`.

## 10. Black-box test matrix (both `funzzy` and `fzz`)

Both binary aliases must expose the identical tree and behavior.

### Parsing & help
- `--help` → stdout, exit 0, lists all subcommands and owned options.
- `-V` / `--version` → stdout version, exit 0.
- `<sub> --help` → subcommand-scoped help.
- unknown option → stderr, exit 2.
- irrelevant option/subcommand combo → stderr, exit 2.
- empty explicit value (`--config=`) → stderr, exit 2 (no silent fallback).

### Watch
- `fzz` (no args, valid config) → runs `run_on_init` tasks.
- `fzz` (no args, no config, no stdin) → guidance, exit 1.
- `fzz watch TARGET` → only matching tasks.
- `fzz watch NOMATCH` → error, exit 1.
- `fzz run TARGET` → finite local execution, combined outcome exit, no watcher/socket.
- `fzz run TARGET PATH` → usage error; path filtering is unsupported.
- `fzz` with piped stdin and no `exec` → usage error.

### exec
- `echo src/a | fzz exec -- echo {{relative_path}}` preserves child argv.
- missing `--` / missing program → exit 2.
- child non-zero → reported failure.
- explicit shell (`... exec -- sh -c '...'`) works.

### init
- `fzz init` creates `.watch.yaml`.
- `fzz init --migrate` wraps legacy list, content/comments preserved.
- `fzz init --wait` → exit 2 (irrelevant).

### list / explain
- `fzz list` prints stable task identity + triggers.
- `fzz explain PATH` prints matched and ignored tasks, deterministic, no execution.
- `fzz explain` with no path → exit 2.
- unmatched path → informative `unmatched`.

### control
- `fzz control status` → compact state; unavailable socket → error + path.
- `fzz control list` → remote targets.
- `fzz control run TARGET` → generation; `--wait` follows exact generation.
- `fzz control run TARGET --timeout 1ns` → deterministic timeout exit.
- `fzz control emit PATH` → matched tasks and run identity; ignored path → no generation.

### Policy
- `--on-busy wait` completes active run before replacement.
- `--on-busy restart` cancels active child, newest generation wins.
- `--control-socket` implies restart.
- `FUNZZY_NON_BLOCK` → restart; `FUNZZY_BAIL` → fail-fast.

## 11. Migration

| V1 | V2 | Note |
| --- | --- | --- |
| `fzz -V` (verbose) | `fzz -v` | short flag swap |
| `fzz -v` (version) | `fzz -V` | short flag swap |
| `fzz --verbose` (rejected) | `fzz --verbose` accepted | fix bug |
| `fzz --non-block` | `fzz --on-busy restart` (or `--restart`) | rename |
| `fzz --target` (value-less lists) | `fzz list` | split discovery from selection |
| `fzz --target @x` | `fzz watch @x` | first-class target |
| `fzz '<command>'` | `fzz exec -- <command...>` | argv boundary |
| `fzz watch '<command>'` | `fzz exec -- <command...>` | unambiguous ad-hoc |
| `fzz --migrate` (without init) | `fzz init --migrate` | scoped flag |
| `--config=` silent fallback | rejected, exit 2 | no silent default |

## 12. Out of scope for V2

- Shell completion generation (deferred with rationale).
- Interactive TUI job switching (bacon-style).
- Generic IPC event bus (only path-shaped `emit`).
- New ignore-vcs/filter flag surface (watchexec-style) — Funzzy's edge is configured workflows, not ad-hoc filtering.
