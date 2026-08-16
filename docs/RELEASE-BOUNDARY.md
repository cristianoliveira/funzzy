# Funzzy v2.0.0 Release Boundary

> Status: **normative** — defined by TASK-0060. Drives TASK-0061 (version identity), TASK-0062 (candidate), TASK-0063 (publication), TASK-0064 (post-publish verification).
> Source: `.tmp/reports/14-08-26/v2-version-bump-plan.md`, current repository state.

## 1. Version decision

The next public release is **`2.0.0`**, not `1.7.0`.

Reason: the current `develop` line introduces intentional public CLI breaks —
real subcommands, removed/renamed flags (`--non-block`, `--target`, `-V`
semantics), command grammar and exit behavior — while preserving selected
compatibility surfaces (both binary names, zero-argument configured watch,
accepted legacy YAML, additive control-protocol fields). SemVer major
communicates this honestly.

Current repository state (the inconsistency this boundary resolves):

| Surface | Value |
|---|---|
| Latest git tag | `v1.6.0` |
| `Cargo.toml` / `Cargo.lock` package | `2.0.0` (candidate) |
| `nix/package.nix` | `1.5.0` (updates at publication, TASK-0063) |
| README | documents V2 behavior |
| pi-watcher package | private `0.1.0` — a compatibility consumer, not a Funzzy release version |

## 2. Scope matrix

### In scope for 2.0.0 (mandatory)

- Real Clap subcommands and global flags (TASK-0015), run/list/watch/explain/exec/init.
- Parallel execution with named contiguous groups, barriers, `on.concurrency`
  (TASK-0024..0029).
- Control socket: status/targets/run/emit/await/cancel/output/capabilities,
  `ctl` alias (TASK-0021..0023, 0043..0048, 0070..0074).
- Agent output evidence: exact output references in correlated snapshots,
  typed instance-exact errors, paging/tail bounds, bounded retrieval
  (TASK-0079..0083; OUTPUT-EVIDENCE-CONTRACT).
- Config hot reload: valid saves swap jobs/roots/policy in-process (PID +
  instance token preserved, revisions monotonic); invalid saves are a
  graceful fatal exit with a terminal error (TASK-0088..0092;
  CONFIG-RELOAD-CONTRACT).
- Duration estimates (TASK-0051..0056).
- Structured control output `--format toon|json|human` (TASK-0048).
- NDJSON run events `--events FILE` (TASK-0039).
- Configurable debounce `on.debounce` and `{{paths}}` (TASK-0031).
- `fzz check` validation and `explain` topology (TASK-0033..0034).
- Preferred `jobs:` vocabulary with deterministic migration (TASK-0075..0078).
- Agent end-to-end feedback loop (TASK-0049).

### Explicitly deferred (post-2.0.0)

- Managed long-running service tasks (TASK-0035), gitignore precedence
  (TASK-0036), polling fallback (TASK-0037), workflow hooks (TASK-0040),
  task-aware output policies (TASK-0041).
- V2 documentation revamp pages beyond release-blocking essentials
  (TASK-0066/0067 deep dives).
- Any runtime/protocol rename of "task" (JOBS-CONFIG-CONTRACT §7).

### Remains compatible (unchanged behavior)

- `funzzy` and `fzz` binary names.
- Zero-argument configured watch.
- Legacy root-list and grouped `on:`/`tasks:` YAML (accepted; not preferred).
- Additive JSON-RPC fields: old clients ignore new keys.
- `{{filepath}}` template (backward compatible; `{{paths}}` is additive).

## 3. Go/no-go gates for the candidate (TASK-0062)

The candidate cut requires each of these satisfied, or an explicit recorded
scope reduction before cut:

| Gate | Status at boundary |
|---|---|
| CLI publication proof (TASK-0020) | requires TASK-0069 (docs drift CI) |
| Parallel performance/lifecycle proof (TASK-0029) | **done** |
| Agent edit feedback loop (TASK-0049) | **done** |
| Duration estimates persisted/bounded (TASK-0056) | **done** |
| Agent configure/validate/run loop (TASK-0059) | requires TASK-0057/0058 |
| Packaging, migration, security/license checks | TASK-0062 evidence |

## 4. Supported environment and install matrix

- Minimum supported Rust version: the Clap-4.6 toolchain (see `nix/` and CI).
- Install channels: Cargo (`cargo install funzzy`), crates.io, GitHub release
  binaries, Nix (stable package), source builds.
- Config formats: preferred `jobs:` list, accepted legacy task forms.
- Protocol/schema versions: declared in `capabilities` (protocolVersion,
  schemaVersion); additive evolution only.
- pi-watcher compatibility: the extension consumes the control socket and
  must pass its suite (unit + real-server one-hop E2E, TASK-0022/0084)
  against the released funzzy.

## 5. Version lifecycle

```text
candidate commit (2.0.0) -> dry-run publish -> tag v2.0.0 (immutable)
  -> GitHub release -> crates.io publish -> stable Nix update -> verify
```

- Ownership: planning and CI **cannot** publish implicitly. The irreversible
  step (TASK-0063) requires exact SHA approval from the maintainer.
- Tags and releases are immutable; never amended, moved, or recreated.
- Partial release resumes channel-aware; never blindly republishes a channel
  that already succeeded.

## 6. Roll-forward policy

- Defects after release produce `2.0.1` (or a withdrawn artifact with an
  incident note). Immutable tags are never rewritten.
- Verification after publish uses fresh install locations, never local build
  outputs (TASK-0064).

## 7. Release notes (2.0.0 candidate)

Breaking CLI grammar/flags/exit behavior (migration: docs/MIGRATION.md):

- Real subcommands (`watch`/`list`/`run`/`explain`/`check`/`config`/`exec`/`control`); removed `--target`/`-t` → `watch TARGET`/`run TARGET`; removed `--non-block`/`-n` → `--on-busy restart`; `exec` preserves argv; exit codes stable (0 success, 1 workflow/operational, 2 usage).

Preserved compatibility:

- `funzzy` and `fzz` binary names; zero-argument configured watch; legacy root-list and grouped `tasks:` YAML accepted (preferred `jobs:` emitted); additive control-protocol fields.

New in 2.0.0:

- Parallel execution: named contiguous groups, barriers, `on.concurrency`.
- Control socket: `control`/`ctl`, status/list/run/emit/await/cancel/output/capabilities, `--format toon|json|human`.
- Agent feedback: exact-generation await, freshness, bounded evidence, NDJSON `--events`, capability negotiation.
- Agent output evidence: exact output references (instance token + generation + optional task), typed instance-exact errors, tail/paging bounds under the transport cap; `output` retrieves bounded evidence in one call (OUTPUT-EVIDENCE-CONTRACT).
- Config reload: valid hot reload keeps the process alive and preserves PID, instance token, and monotonic revisions; a formatting-only save is a no-op; an invalid config exits nonzero with a terminal gate/reason (`configInvalid` to subscribers); a deleted config is fatal; managed services reconcile at the commit boundary (CONFIG-RELOAD-CONTRACT).
- Duration estimates (XDG history, execution-signature keyed).
- Configuration discovery: `fzz config schema|example`, `fzz check`, `fzz explain`.
- Watcher: `on.debounce`, `on.watch_backend` (native/poll/auto), `on.respect_gitignore`.
- Shell completion: `fzz completions SHELL`.

Deferred: service tasks (TASK-0035), workflow hooks (TASK-0040), task-aware output policies (TASK-0041), docs deep dives.

Release evidence locked by tests (TASK-0020): `funzzy`/`fzz` identical command trees; removed V1 flags rejected with exit 2; every subcommand `--help` exits 0.

Release evidence locked by tests (TASK-0020): `funzzy` and `fzz` expose
identical command trees; removed V1 flags (`--non-block`, `--target`, `-n`,
`-t`) are rejected with exit 2; every subcommand `--help` exits 0; exit codes
are stable (0 success/no-op, 1 workflow/operational, 2 usage).
