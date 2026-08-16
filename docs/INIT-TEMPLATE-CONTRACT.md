# Funzzy Init Template Contract

> Status: **normative** — defined by TASK-0093. Drives TASK-0094 (canonical option catalog + renderer) and TASK-0095 (black-box proof and drift gate).
> Source: current parser (`src/config.rs`), `src/cli/init.rs`, `src/cli/config.rs`, AGENT-CONFIG-CONTRACT (TASK-0057), JOBS-CONFIG-CONTRACT (TASK-0075), GITIGNORE-CONTRACT, PARALLEL-EXECUTION-CONTRACT, SERVICE-TASKS-CONTRACT, OUTPUT-POLICY-CONTRACT, RUN-HOOKS-CONTRACT.

## 1. Purpose

`fzz init` today writes a small runnable `.watch.yaml` that omits most supported settings, forcing users to search the docs and letting parser/schema knowledge and generated configuration drift apart. The redesigned template follows the TypeScript `tsconfig.json` pattern: **a small active setup plus commented discoverable options**. One generated file is both an immediately runnable starter and a bounded, deterministic index of every supported configuration field.

Success: a user can run `fzz init && fzz` immediately, and the same file acts as a complete commented configuration reference.

## 2. Scope of the generated file

The template contains **only** the preferred V2 root vocabulary: an ordered `jobs:` list with an optional shared `on:` block. The inventory in §3 is the complete, normative list.

| In scope | Out of scope (explicitly excluded) |
|---|---|
| `on:` properties (§3.2) | Legacy root task list (`- name: ...`) |
| `jobs[]` properties (§3.3) | Legacy `tasks:` root key and migration prose |
| Command template variables (§3.4) | CLI-only controls (`--fail-fast`, `--log-file`, `--events-file`, `--sequential`, `--on-busy`, `--restart`, `--no-run-on-init`, `--target`, `--verbose`) |
| Next-command hints (`fzz check`, `fzz list`, `fzz run`, `fzz watch`, `fzz config schema`, `fzz config example minimal`) | Conceptual schema sections (`matching`, `execution`, `parallel`, `control`) as config keys |
| | Project/toolchain-specific commands (no `cargo`/`npm`/`git` starter commands) |

CLI-only controls are named in next-command comments only; they must never appear as `.watch.yaml` keys. Conceptual schema sections are a discovery aid (`fzz config schema --section S`), not config syntax, and cannot masquerade as config keys in the template.

## 3. Complete supported-property inventory

This is the normative inventory the template comments must cover and the TASK-0094 catalog must own. It was extracted from the production parser, not from the schema (the schema currently lags — see §9).

### 3.1 Root

| Property | Required | Type / shape | Semantics |
|---|---|---|---|
| `on` | no | object | Shared settings merged into every job (§6). |
| `jobs` | **yes** | ordered array of objects, ≥ 1 | Declared workflow units. Mapping form, empty list, and duplicate `name`s are errors. Mixed `jobs:` + `tasks:` is an error, never a silent merge. |

### 3.2 `on:` properties (11)

| Property | Required | Default | Type / allowed values | Purpose |
|---|---|---|---|---|
| `change` | no | `[]` | string \| array of glob strings | Common change globs inherited by every job (merged, common-first, deduped). |
| `ignore` | no | `[]` | string \| array of glob strings | Common ignore globs inherited by every job; explicit config ignore wins over gitignore. |
| `socket` | no | none (off) | string (path) | Control socket path; enables the control surface (`fzz control`). |
| `concurrency` | no | available parallelism | integer ≥ 1 | Global cap on simultaneously active tasks. |
| `debounce` | no | `1s` | duration `<number>` (seconds) or `<number>ms\|s\|m` | Filesystem batch debounce window. |
| `watch_backend` | no | `auto` | enum `native` \| `poll` \| `auto` | Watch backend selection; `auto` tries native then poll. |
| `poll_interval` | no | `500ms` | duration | Poll backend interval (only meaningful with `poll`). |
| `respect_gitignore` | no | `false` | boolean | Respect workspace `.gitignore` rules. |
| `success` | no | none | string (command) | Hook run after a successful generation. |
| `failure` | no | none | string (command) | Hook run after a failed generation. |
| `output` | no | `inherit` | enum `inherit` \| `quiet` \| `capture` \| `show-on-failure` | On-level default output policy. |

### 3.3 `jobs[]` properties (10)

| Property | Required | Default | Type / allowed values | Purpose |
|---|---|---|---|---|
| `name` | **yes** | — | string | Stable job identity; also the runtime task name. Unique across the file. |
| `run` | **yes** | — | string \| array of strings | Command(s): a shell string or an argv list. May contain template variables (§3.4). |
| `change` | no | inherited | string \| array of glob strings | Trigger globs; appended to and deduped against `on.change`. |
| `ignore` | no | inherited | string \| array of glob strings | Suppression globs; appended to and deduped against `on.ignore`; strongest precedence. |
| `run_on_init` | no | `false` | boolean | Run this job when the watcher starts. |
| `parallel` | no | none | string (group name) | Named contiguous group; members run concurrently within the concurrency cap. |
| `cwd` | no | workspace root | string (path) | Working directory for this job, relative to the workspace root. |
| `env` | no | inherited | map string → string | Per-job environment. Values are never echoed by config commands and never appear in generated files. |
| `service` | no | `false` | boolean | Managed long-running service: started on init, restarted on change, stopped on exit. |
| `output` | no | `on.output` or `inherit` | enum `inherit` \| `quiet` \| `capture` \| `show-on-failure` | Job-level override of the output policy. |

### 3.4 Command template variables

Available inside `run` (shell string and argv form); unknown variables are reported, not silently dropped.

| Variable | Expansion |
|---|---|
| `{{filepath}}` | The triggering path (backward-compatible). |
| `{{absolute_path}}` | Alias of `{{filepath}}`. |
| `{{relative_filepath}}` / `{{relative_path}}` | The triggering path relative to the workspace root. |
| `{{paths}}` | The complete normalized changed-path set of the triggering batch, shell-escaped and space-joined (empty for runs without a batch). |

### 3.5 Excluded supported inputs (documented, never generated)

Legacy root list and grouped `on:`/`tasks:` forms remain accepted by the parser (compatibility window, JOBS-CONFIG-CONTRACT §4) but are never emitted by `fzz init`, `fzz init --migrate` output, or `fzz config example`.

## 4. Active starter contract

The generated file must be immediately runnable in any directory, with no Cargo/npm/language/toolchain dependency. The active (non-commented) part stays small and behaviorally equivalent to today's starter:

1. `hello world` — `run: echo "Funzzy hello world! Next step, add rules into .watch.yaml"`, `run_on_init: true`. Proves `fzz` runs a job on init.
2. `list files` — `run: 'ls -a'`, `change: '**/*.txt'`, `ignore: '**/*.log'`. Proves change matching with ignore; works in any directory.
3. `on.socket: .tmp/funzzy/control.sock` stays active: the control surface is currently useful and documented in the same file.

Nothing else is active. Every other supported property appears only as a commented reference (§5). Uncommenting any documented scalar example must yield parser-valid YAML.

## 5. Commented reference contract

- **One owner section**: `on:` properties are commented under `on:`; job properties under `jobs:` (as comments beside a commented example job or inline above the active jobs). A property appears exactly once.
- **Comment anatomy** — each optional property comment carries, in order:
  1. one-line purpose;
  2. default when meaningful (enums, booleans, durations, policies);
  3. allowed values for enums;
  4. shape/example where ambiguity exists (string vs array, argv form, env map, template variables).
  Fields with an obvious scalar shape need only purpose (+ default when non-obvious).
- **Parser-valid examples**: every example value is chosen from the allowed enum/default set; uncommenting the example line produces a config `fzz check` accepts.
- **No secrets**: comments and examples never include secret-like values; the `env` example uses a harmless placeholder such as `FOO: bar`.
- **Plain YAML**: only `#` comments; no terminal-width tricks, no ASCII-art alignment beyond stable indentation.

## 6. Semantics explained in the file

The template explains, briefly and in comments (not a manual):

- `on.change`/`on.ignore` are **inherited**: merged into each job (common first, deduped); a job's own patterns extend them, never replace them.
- `on.ignore` beats gitignore when both apply; explicit config ignore has strongest precedence.
- **Declaration order is semantic**: jobs run in order; contiguous jobs sharing a `parallel:` group run concurrently within the concurrency cap (occurrence = contiguous span, PARALLEL-EXECUTION-CONTRACT). Jobs do **not** form a DAG.
- Required fields: `jobs` (non-empty) and each job's `name` (unique) + `run`.

## 7. Next-command documentation

The template header documents the immediate next commands, exactly as installed:

```text
fzz check                     validate this file (no watcher)
fzz list                      show configured jobs and their change patterns
fzz run <name>                run one job once, locally
fzz / fzz watch               start watching (both spellings work)
fzz control status            talk to a running watcher (socket enabled)
fzz config schema             full field reference from the installed binary
fzz config example minimal    tiny machine-copyable alternative starter
```

`fzz config schema` is called out as the installed authoritative reference; the file is an index, not a replacement for it.

## 8. Determinism and size budget

- **Deterministic bytes**: the template is a compile-time constant. No terminal width, environment variable, repository content, user/host, timestamp, or network access influences it. Identical bytes on every run, every machine.
- **Stable ordering**: fixed section order — header (purpose + next commands) → `on:` block → `jobs:` block (reference comments first, active starter last) → no trailing content.
- **Size budget**: hard ceiling **200 lines / 8 KiB**; the design target is ≈120 lines. The exact bytes are frozen by the golden snapshot in TASK-0095; the TASK-0069 drift gate fails when the installed binary's output diverges from the snapshot. Raising the ceiling requires a design review that updates this contract.
- **No `--minimal` flag**: `fzz config example minimal` already provides a concise machine-copyable alternative; a new `fzz init --minimal` flag is out of scope unless separately evidenced.

## 9. Known drift exposed by the inventory

The inventory surfaced parser/schema gaps that TASK-0094 must close via the canonical catalog (§10); the contract records them so the catalog tests can target them:

1. **Parser accepts but schema/template omit**: `on.success`, `on.failure`, `on.output`, job `service`, job `output` are parsed and enforced by `src/config.rs` but missing from `src/cli/config.rs` schema sections.
2. **Stale error text**: the `on`-section unknown-property error message lists only eight allowed properties, omitting `success`, `failure`, `output` (the check itself accepts them).
3. **Job-level unknown keys are silently accepted**: `fzz check` validates a config with an unknown job property (`bogus_key: 1` passes) although the schema declares `additionalProperties: false` and JOBS-CONFIG-CONTRACT §5 expects actionable validation.
4. **Schema pseudo-fields**: `matching`/`execution`/`parallel`/`control` schema sections are conceptual (CLI/protocol), not legal YAML properties; schema structural definitions must never be read as config keys.

None of these block this contract; each is a TASK-0094 acceptance criterion.

## 10. Single source of truth

One canonical option catalog (TASK-0094) owns property identity, default, type/enum, and help for every field in §3. It drives the commented init renderer **and** `fzz config schema`; parser allowlists/error messages consume it or are enforced by parity tests. The template, the schema, and the parser are derived views of the same metadata — this contract defines what must be derivable, not a second hand-maintained copy.

## 11. Out of scope

- Legacy migration prose inside the generated file (migration lives in `fzz init --migrate` and JOBS-CONFIG-CONTRACT).
- Project/toolchain-specific starter commands.
- A `fzz init --minimal` flag.
- Replacing `fzz config schema` as the authoritative discovery surface.
