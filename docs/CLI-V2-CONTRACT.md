# Funzzy V2 CLI Contract

> Status: **draft** — defined by TASK-0014. Drives TASK-0015 through TASK-0024.
> Configuration-command responsibilities: **normative** — defined by TASK-0096. Drives TASK-0097, TASK-0098, TASK-0099.
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
| `fzz init [--template PROFILE]` | Create `.watch.yaml` starter (create-only) | no |
| `fzz migrate` | Rewrite legacy config to preferred `jobs:` form in place | no |
| `fzz exec [options] -- PROGRAM [ARG...]` | Ad-hoc watch over stdin-supplied paths | **yes** |
| `fzz control status` | Print running watcher state over Unix socket | no |
| `fzz control list` | Print remote targets from running watcher | no |
| `fzz control run TARGET [--wait] [--timeout DUR]` | Trigger named target; optionally await terminal outcome | no |
| `fzz control emit PATH [--wait] [--timeout DUR]` | Submit synthetic path change through native routing | no |

`ctl` is a visible Clap alias for the canonical `control` subcommand (TASK-0070). Every nested operation, option, and exit code is identical under both spellings; `control` remains the documentation and protocol vocabulary. No aliases exist for nested operations.

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

- `init` owns `--template <comprehensive|minimal|parallel|agent>` (default `comprehensive`).
- `migrate` accepts the global `-c, --config <FILE>` to select the file it rewrites.
- `config` owns `--section`, PROFILE positional, and `--format`.
- `control` owns `--socket <PATH>`, `--wait`, `--timeout <DUR>`.
- `exec` owns the trailing `-- PROGRAM [ARG...]`.
- `watch` owns the optional positional `TARGET`.

Irrelevant option/subcommand combinations (e.g., `init --wait`, `list --on-busy restart`, `init --migrate`) **fail explicitly** with a usage error. `--migrate` is not a V2 flag: migration is the explicit `fzz migrate` subcommand (§3a).

## 3a. Configuration command responsibilities

> Normative — TASK-0096. One responsibility per command name; command names
> predict side effects. Drives TASK-0097 (template ownership) and
> TASK-0098 (`fzz migrate`).

| Command | Responsibility | Reads project config? | Writes filesystem? | stdout payload |
| --- | --- | --- | --- | --- |
| `fzz init [--template P]` | **create** a starter config | no | `.watch.yaml` (create-only; refuses existing) | one-line success notice |
| `fzz config schema [--section S]` | **describe** the installed config contract | no | none | JSON Schema / bounded section |
| `fzz config example PROFILE` | **export** profile artifact bytes | no | none | deterministic YAML bytes |
| `fzz migrate [-c FILE]` | **transform** legacy config → preferred `jobs:` | source file only | atomic in-place rewrite | one-line outcome |
| `fzz check [-c FILE]` | **validate** the selected config | yes | none | validation diagnostics |

### `fzz init` — create-only

- Writes `.watch.yaml` in the current working directory. It never reads,
  merges, overwrites, or validates an existing config.
- **Deterministic refusal**: if the destination exists, fail (exit 1) with a
  stable message; the existing file's bytes are untouched. There is no
  `--force`/overwrite path — overwrite is deliberately not a responsibility.
- `--template comprehensive|minimal|parallel|agent` (default `comprehensive`):
  selects one of the shared profile artifacts (INIT-TEMPLATE-CONTRACT owns
  `comprehensive`; AGENT-CONFIG-CONTRACT owns `minimal`/`parallel`/`agent`).
  Default `comprehensive` keeps `fzz init && fzz` generic and runnable.
- Invalid profile is a usage error (exit 2) naming the valid values.
- Emitted bytes are deterministic: identical on every run and machine.

### `fzz config schema|example` — side-effect-free

- Stdout-only, single document, no filesystem writes, no watcher, no socket.
- `config example PROFILE` accepts the same four profiles as `init` and
  prints **byte-identical** artifact bytes (`fzz init --template P` writes
  exactly what `fzz config example P` prints).
- `config example` never gains file-writing flags (`-o`, `--write`, …):
  piping remains the agent surface (`fzz config example minimal > .watch.yaml`).

### `fzz migrate` — explicit transform

- Migrates the file selected by global `-c, --config` (default `.watch.yaml`).
- Accepted inputs: legacy root task list (wrapped under `jobs:`, order and
  comments preserved), grouped `on:`/`tasks:` (root key renamed to `jobs:`),
  and already-preferred `jobs:` (byte-identical no-op, exit 0).
- Errors (exit 1, original bytes unchanged): missing file, malformed YAML,
  multiple documents, unsupported root shape, empty task list.
- Write is **atomic** (same-directory temp file + rename); failure at any
  point leaves the original untouched. A successful migration's output must
  pass `fzz check` (proved in TASK-0099).
- One-line outcome on stdout; diagnostics on stderr; deterministic output.
- `fzz init --migrate` is **removed** from the V2 contract — V2 is an
  intentional breaking boundary, no deprecated alias is carried.

### Why the generator overlap is intentional

`init` and `config example` emit the same per-profile artifacts by design:
  same bytes, different **destination** (file vs stdout) and different **user
  intent** ("set up this directory" vs "show me the bytes to copy/pipe").
  Drift is impossible because one option catalog (TASK-0094) owns the bytes.

### `fzz check` — no conflict

`check` validates the **existing selected** config; it never creates,
  describes, exports, or transforms. The four commands above never validate —
  `check` is the single validation entry point.

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

> Normative identity, state, freshness, await, evidence, and exit-code contract: `docs/AGENT-FEEDBACK-CONTRACT.md` (TASK-0042). This section states CLI-visible behavior; the contract owns protocol semantics.

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

### init / migrate
- `fzz init` creates `.watch.yaml` (comprehensive template).
- `fzz init --template minimal|parallel|agent` writes that profile's bytes; invalid profile → exit 2 with valid values.
- `fzz init` with existing `.watch.yaml` → exit 1, bytes untouched.
- `fzz init --wait` / `fzz init --migrate` → exit 2 (irrelevant/unknown flag).
- `fzz migrate` wraps legacy list (comments preserved), renames `tasks:` → `jobs:`, no-ops on `jobs:`.
- `fzz migrate` on missing/malformed/unsupported file → exit 1, original unchanged.
- `fzz migrate -c custom.yml` migrates that file.

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
| `fzz --migrate` (without init) | `fzz migrate` | explicit subcommand |
| `fzz init --migrate` | `fzz migrate` | one responsibility per command |
| `--config=` silent fallback | rejected, exit 2 | no silent default |

## 12. Out of scope for V2

- Shell completion generation (deferred with rationale).
- Interactive TUI job switching (bacon-style).
- Generic IPC event bus (only path-shaped `emit`).
- New ignore-vcs/filter flag surface (watchexec-style) — Funzzy's edge is configured workflows, not ad-hoc filtering.
