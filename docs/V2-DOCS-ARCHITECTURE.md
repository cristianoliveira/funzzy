# Funzzy V2 documentation audit and information architecture

> Status: **done** — defined by TASK-0065. Drives TASK-0066 (rewrite onboarding/config), TASK-0067 (advanced guides), TASK-0068 (cleanup/migration/examples), TASK-0069 (drift CI), and TASK-0075 (jobs vocabulary).
> Source of truth for this audit: current Clap help (`fzz --help`, `fzz watch --help`), `src/arguments.rs`, `src/config.rs`, and the normative contracts.

## 1. Page inventory

Every current README/docs/examples page classified with owner, audience, source of truth, and V2 readiness.

| Page | Verdict | Audience | Source of truth | V2 ready? | Action owner |
|---|---|---|---|---|---|
| `README.md` | **rewrite** (bounded orientation) | users | current CLI + config | partial — teaches removed `--non-block` nowhere but lacks batching/parallel/control docs (added in TASK-0028/0029/0031) | TASK-0066 |
| `docs/USAGE.md` | **rewrite → getting-started + daily-workflows guide** | users | current CLI + config | **no** — teaches removed `--non-block` (lines 289–296, 346–348), `--target` implicitly, V1.6 version claims | TASK-0066 |
| `docs/CLI-V2-CONTRACT.md` | **keep** (normative) | contributors | Clap definitions in `src/arguments.rs` | yes (updated through TASK-0070) | TASK-0069 drift gate |
| `docs/AGENT-FEEDBACK-CONTRACT.md` | **keep** (normative) | contributors/agents | `src/control.rs`, `src/snapshot.rs` | yes | TASK-0069 drift gate |
| `docs/PARALLEL-EXECUTION-CONTRACT.md` | **keep** (normative) | contributors | `src/executor.rs`, `src/plan.rs` | yes | TASK-0069 drift gate |
| `docs/RUN-DURATION-ESTIMATES-CONTRACT.md` | **keep** (normative) | contributors | `src/duration_*.rs` | yes | TASK-0069 drift gate |
| `docs/SEQUENTIAL-OVERRIDE-CONTRACT.md` | **keep** (normative) | contributors/agents | implementation | yes | TASK-0069 drift gate |
| `docs/WATCH-DISCOVERY-CONTRACT.md` | **keep** (normative) | contributors | `src/watches.rs`, `src/watcher.rs`, `src/watch_loop.rs` | yes (defined by TASK-0085) | TASK-0086/0087 drift gate |
| `docs/CONFIG-RELOAD-CONTRACT.md` | **keep** (normative) | contributors | `src/app.rs`, `src/config.rs`, `src/process_owner.rs` | yes (defined by TASK-0088) | TASK-0089..0092 drift gate |
| `docs/INIT-TEMPLATE-CONTRACT.md` | **keep** (normative) | contributors/agents | `src/config.rs`, `src/cli/init.rs`, `src/cli/config.rs` | yes (defined by TASK-0093) | TASK-0094/0095 drift gate |
| `docs/DURATION-ESTIMATES-GUIDE.md` | **keep** (user guide) | users/agents | contract + control surface | yes | TASK-0067 polish |
| `docs/FLAG_NON_BLOCK.md` | **archive/delete** | users | removed V1 flag `--non-block` → `--on-busy restart` | **no** — teaches removed vocabulary | TASK-0068 |
| `docs/FLAG_TARGET.md` | **archive/delete** | users | removed V1 flag `--target` → `watch TARGET`/`list` | **no** — teaches removed vocabulary | TASK-0068 |
| `docs/FLAG_CONTROL_SOCKET.md` | **rewrite → control guide** | users/agents | `control` subcommand + socket config | partial — stale V1 prose, missing `ctl` alias/run/emit/await/cancel/output | TASK-0067 |
| `docs/FLAG_FAIL_FAST.md` | **rewrite → workflow semantics page** | users | `--fail-fast` + config | partial | TASK-0067 |
| `docs/FLAG_LOG_FILE.md` | **rewrite → workflow semantics page** | users | `--log-file` + config | partial | TASK-0067 |
| `docs/FLAG_NO_RUN_INIT.md` | **rewrite → workflow semantics page** | users | `--no-run-on-init` + config | partial | TASK-0067 |
| `examples/*.yml` (12 files) | **keep + verify** | users (copyable) | current parser | mixed — several exercise removed `--non-block` flows or legacy formats | TASK-0068 |
| `examples/longtask.sh` | **rewrite** | users | `--on-busy restart` | **no** — comment teaches `--non-block` | TASK-0068 |
| `AGENTS.md`, `CLAUDE.md` | **keep** (contributor orientation) | contributors | repo | yes | none |

## 2. Stale-claim evidence (audited against current implementation)

Confirmed stale or removed vocabulary still taught live:

- `--non-block` / `-n`: removed V2 flag (rejected by parser, `src/arguments.rs:1320`); replacement is `--on-busy restart` / `--restart`. Still taught in `docs/USAGE.md` (289–296, 346–348), `docs/FLAG_NON_BLOCK.md` (whole page), `examples/longtask.sh` (comment).
- `--target` / `-t`: removed V2 flag (rejected, `src/arguments.rs:1290`); replacement is `watch TARGET` / `list` / `run TARGET`. Still taught in `docs/FLAG_TARGET.md` (whole page).
- Version claims: `docs/USAGE.md` cites "Minimal version v1.6.0" and earlier V1 versions; current package is 2.0.0-beta.1 (see TASK-0060 for the release-boundary decision).
- `docs/FLAG_CONTROL_SOCKET.md` predates the full `control` subcommand tree (status/list/run/emit/await/cancel/output/capabilities), the `ctl` alias (TASK-0070), and correlated snapshots.

## 3. Target information architecture

One navigation with one obvious route per audience and topic; README is orientation, not a full manual.

```text
README.md                          # orientation: what/install/quick-start/navigation
├── Getting started & daily workflows   (was docs/USAGE.md, rewritten)
│   ├── configuration (.watch.yaml, on.*, tasks, groups, jobs→V2)
│   ├── daily commands (watch/list/run/explain/exec/init)
│   └── parallel execution & event batching
├── Advanced operations (TASK-0067)
│   ├── control socket & ctl alias
│   ├── duration estimates
│   ├── failure semantics (fail-fast, restart policy)
│   ├── log file
│   └── sequential debugging / agents
├── Migration (TASK-0068): V1 flags → V2 commands table
├── Troubleshooting
└── Reference (generated, TASK-0058/0069)
    ├── command help (from Clap)
    ├── config schema/examples (from parser + fixtures)
    └── protocol capability tables (from capabilities)
```

Rules:

- **README is bounded**: install, quick start, and links. No duplicate full manual.
- **Normative contracts stay separate and labeled**: `docs/*-CONTRACT.md` are contributor/agent reference evidence, clearly not user tutorials.
- **Generated over handwritten**: command help, config schema/examples, protocol tables come from Clap/parser/capabilities; handwritten duplication is minimized (TASK-0069 enforces no drift).
- **Versioning**: docs on `develop` describe unreleased V2 behavior; tagged releases get stable URLs; `on.debounce`/jobs/etc. land only when their implementation lands.
- **Terminology**: one glossary (jobs→tasks/commands boundary lands in TASK-0075); examples use either `control` or `ctl` consistently within a page.
- **Do not expose planning/TASK language** as primary user docs.

## 4. Release-blocking vs deferred

Release-blocking (needed for v2.0.0, TASK-0062 gate):

- README + getting-started/config/daily-workflows rewrite (TASK-0066).
- Removal of stale V1 flag pages and modernized examples (TASK-0068).
- Drift CI for help/schema/examples/links/version/vocabulary (TASK-0069).

Deferred (can follow the release):

- Advanced deep-dives and agent workflows (TASK-0067) — most content is additive, not corrective.
- Normative-contract polish beyond drift enforcement.

## 5. Next-step map

```text
TASK-0065 (this audit)
  ├── TASK-0066 rewrite onboarding/config ← config schema + check (TASK-0033/0058)
  ├── TASK-0067 advanced operations ← parallel output (0028) + estimates (0055) + agent config (0057/0058)
  └── TASK-0068 cleanup/migration/examples ← migration proof (TASK-0078)
        └── TASK-0069 drift CI
              └── TASK-0020 final CLI publication gate
```
