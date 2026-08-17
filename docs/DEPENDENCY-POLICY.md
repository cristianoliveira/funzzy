# Dependency inventory and update policy

Status: normative baseline for TASK-0103 · captured 2026-08-17

This document is the one inventory and update policy for root Cargo/Nix/CI and
the `pi-watcher` npm/Nix submodule. It records registry state without changing
any manifest or lockfile.

## 1. Update classes

Every dependency change must use one class and one reviewable commit:

1. **Compatible lock refresh** — resolved version changes inside unchanged
   manifest constraint. Review exact lock/checksum diff; run focused tests.
2. **Manifest minor update** — declared constraint changes without expected API
   or behavior migration. For `0.x`, treat a minor as breaking unless upstream
   explicitly proves otherwise.
3. **Major/API migration** — isolate adapter/API edits, characterize behavior
   first, and run full affected integration matrix.
4. **Replacement/removal** — prove dependency unused or replacement equivalent;
   remove source/import/lock entries together.
5. **Explicit deferral** — record owner task, reason, advisory impact, and a
   concrete revisit trigger. “Latest” alone is not a reason to update.

Never use blind `cargo update`, `npm update`, or `nix flake update`. Select one
package/input family, inspect proposed lock changes, verify, commit, then move to
next family.

Table actions use: **C** compatible/current, **M** manifest minor, **A**
major/API migration, **R** replacement/removal, **D** explicit deferral.
“Active/legacy line” means upstream is maintained but selected major is old.

## 2. Rust direct dependencies

Exact current versions come from `Cargo.lock` resolve edges, not only manifest
constraints. Latest/license/maintenance use crates.io package metadata; advisory
status uses exact-version OSV/RustSec queries on 2026-08-17.

### Runtime

| Package | Declared | Current | Latest stable | License | Maintenance | Advisory | Action / owner | Source |
|---|---:|---:|---:|---|---|---|---|---|
| serde | 1.0 | 1.0.229 | 1.0.229 | MIT OR Apache-2.0 | active | none | C / 0104 | [crate](https://crates.io/crates/serde) |
| serde_derive | 1.0 | 1.0.229 | 1.0.229 | MIT OR Apache-2.0 | active | none | R: evaluate `serde/derive` / 0104 | [crate](https://crates.io/crates/serde_derive) |
| serde_json | 1.0 | 1.0.151 | 1.0.151 | MIT OR Apache-2.0 | active | none | C / 0104 | [crate](https://crates.io/crates/serde_json) |
| toon | 0.1.2 | 0.1.2 | 0.1.2 | MIT | active, low-volume | none | C / 0104 | [crate](https://crates.io/crates/toon) |
| ignore | 0.4.33 | 0.4.33 | 0.4.33 | Unlicense OR MIT | active | none | C / 0104 | [crate](https://crates.io/crates/ignore) |
| notify | 4.0.0 | 4.0.18 | 8.2.0 | CC0-1.0 | active/legacy line | none | A: unify major / 0105 | [crate](https://crates.io/crates/notify) |
| yaml-rust | 0.4.5 | 0.4.5 | 0.4.5 | MIT OR Apache-2.0 | **unmaintained** | [RUSTSEC-2024-0320](https://rustsec.org/advisories/RUSTSEC-2024-0320.html), informational | D bulk update; R / 0112 | [crate](https://crates.io/crates/yaml-rust) |
| glob | ^0.3 | 0.3.4 | 0.3.4 | MIT OR Apache-2.0 | active | none | C / 0104 | [crate](https://crates.io/crates/glob) |
| notify-debouncer-mini | 0.3.0 | 0.3.0 | 0.7.0 | CC0-1.0 OR Artistic-2.0 | active/legacy line | none | A with notify / 0105 | [crate](https://crates.io/crates/notify-debouncer-mini) |
| nix | 0.26.2 | 0.26.4 | 0.31.3 | MIT | active/legacy line | none | A: signal/process APIs / 0105 | [crate](https://crates.io/crates/nix) |
| once_cell | 1.19 | 1.21.4 | 1.21.4 | MIT OR Apache-2.0 | active | none | R: `LazyLock` if equivalent / 0104 | [crate](https://crates.io/crates/once_cell) |
| clap | 4.6 | 4.6.6 | 4.6.6 | MIT OR Apache-2.0 | active | none | C / 0104 | [crate](https://crates.io/crates/clap) |
| clap_complete | 4.5 | 4.6.9 | 4.6.9 | MIT OR Apache-2.0 | active | none | M: align constraint / 0104 | [crate](https://crates.io/crates/clap_complete) |
| sha2 | 0.10 | 0.10.9 | 0.11.0 | MIT OR Apache-2.0 | active/previous line | none | A only with hash fixtures / 0104 | [crate](https://crates.io/crates/sha2) |

### Development

| Package | Declared | Current | Latest stable | License | Maintenance | Advisory | Action / owner | Source |
|---|---:|---:|---:|---|---|---|---|---|
| assert_cmd | =2.1.2 | 2.1.2 | 2.2.2 | MIT OR Apache-2.0 | active | none | M; retain/explain exact pin / 0104 | [crate](https://crates.io/crates/assert_cmd) |
| predicates | 3.1.0 | 3.1.4 | 3.1.4 | MIT OR Apache-2.0 | active | none | C / 0104 | [crate](https://crates.io/crates/predicates) |
| pretty_assertions | 1.4.1 | 1.4.1 | 1.4.1 | MIT OR Apache-2.0 | maintained/current | none | C / 0104 | [crate](https://crates.io/crates/pretty_assertions) |

`cargo-audit` is not installed in baseline environment. Exact direct-package OSV
queries found no vulnerability advisory and one informational unmaintained
advisory above. TASK-0108 owns reproducible full direct/transitive RustSec
scanning; no unacknowledged high/critical advisory may pass.

### Transitive duplicate majors

`cargo metadata --locked` reports these duplicate families:

| Family | Versions | Why present | Disposition |
|---|---|---|---|
| notify | 4.0.18, 6.1.1 | direct notify 4; debouncer 0.3 pulls notify 6 | remove in 0105 |
| fsevent-sys | 2.0.1, 4.1.0 | two notify lines (macOS) | remove with notify 4 |
| inotify | 0.7.1, 0.9.6 | two notify lines (Linux) | remove with notify 4 |
| mio | 0.6.23, 0.8.11 | two notify lines | remove with notify 4 |
| cfg-if | 0.1.10, 1.0.4 | old mio/net2 vs current dependencies | remove old line in 0105 |
| winapi | 0.2.8, 0.3.9 | old mio/net2 Windows chain | remove old line in 0105 |
| bitflags | 1.3.2, 2.13.1 | nix 0.26/notify 4 vs notify 6/kqueue | reassess after 0105 |
| windows-sys | 0.48.0, 0.61.2 | notify 6/mio vs current CLI utilities | reassess after 0105 |

There is no unexplained duplicate family. TASK-0105 must converge to one notify
major and re-run `cargo tree --duplicates`; remaining target-specific duplicates
must retain an explicit route and owner.

## 3. pi-watcher direct dependencies

Current means exact installed/package-lock version. npm registry metadata reports
no deprecation for any row. `npm audit --audit-level=high` scans full tree and
reports 0 info/low/moderate/high/critical vulnerabilities.

### Runtime and peer

| Kind | Package | Declared | Current | Latest stable | License | Maintenance | Advisory | Action / owner | Source |
|---|---|---:|---:|---:|---|---|---|---|---|
| runtime | yaml | 2.9.0 | 2.9.0 | 2.9.0 | ISC | active | none | C / 0106 | [npm](https://www.npmjs.com/package/yaml) |
| peer | @earendil-works/pi-ai | * | 0.84.1 via dev pin | 0.84.2 | MIT | active | none | M paired / 0106 | [npm](https://www.npmjs.com/package/@earendil-works/pi-ai) |
| peer | @earendil-works/pi-coding-agent | * | 0.84.1 via dev pin | 0.84.2 | MIT | active | none | M paired / 0106 | [npm](https://www.npmjs.com/package/@earendil-works/pi-coding-agent) |
| peer | typebox | * | 1.3.7 via dev pin | 1.3.15 | MIT | active | none | M / 0106 | [npm](https://www.npmjs.com/package/typebox) |

Peer `*` ranges are intentional because host Pi supplies runtime modules. Exact
dev pins and package lock define tested compatibility; Pi packages must not move
from peer to bundled runtime dependencies.

### Development/tooling

| Package | Declared/current | Latest stable | License | Maintenance | Advisory | Action / owner | Source |
|---|---:|---:|---|---|---|---|---|
| @earendil-works/pi-ai | 0.84.1 | 0.84.2 | MIT | active | none | M paired / 0106 | [npm](https://www.npmjs.com/package/@earendil-works/pi-ai) |
| @earendil-works/pi-coding-agent | 0.84.1 | 0.84.2 | MIT | active | none | M paired / 0106 | [npm](https://www.npmjs.com/package/@earendil-works/pi-coding-agent) |
| @eslint/js | 10.0.1 | 10.0.1 | MIT | active | none | C / 0106 | [npm](https://www.npmjs.com/package/@eslint/js) |
| @types/node | 26.2.0 | 26.2.0 | MIT | active | none | C; prove Node 24 / 0106 | [npm](https://www.npmjs.com/package/@types/node) |
| @vitest/coverage-v8 | 4.1.10 | 4.1.10 | MIT | active | none | C with Vitest / 0106 | [npm](https://www.npmjs.com/package/@vitest/coverage-v8) |
| eslint | 10.8.1 | 10.8.1 | MIT | active | none | C / 0106 | [npm](https://www.npmjs.com/package/eslint) |
| prettier | 3.9.6 | 3.9.6 | MIT | active | none | C / 0106 | [npm](https://www.npmjs.com/package/prettier) |
| typescript | 5.9.3 | 7.0.2 | Apache-2.0 | active/previous line | none | **D** ecosystem cap / 0106 | [npm](https://www.npmjs.com/package/typescript) |
| typebox | 1.3.7 | 1.3.15 | MIT | active | none | M / 0106 | [npm](https://www.npmjs.com/package/typebox) |
| typescript-eslint | 8.67.0 | 8.67.0 | MIT | active | none | C / 0106 | [npm](https://www.npmjs.com/package/typescript-eslint) |
| vitest | 4.1.10 | 4.1.10 | MIT | active | none | C with coverage / 0106 | [npm](https://www.npmjs.com/package/vitest) |

**TypeScript 7 decision:** defer. `typescript-eslint@8.67.0` declares TypeScript
`>=4.8.4 <6.1.0`; therefore TS 7 is outside supported ecosystem even though
Vitest and Pi packages do not add a conflicting TypeScript peer constraint.
TASK-0106 may revisit only when TypeScript-ESLint supports TS 7 and the complete
Node 24 + ESLint + Vitest/coverage + Pi SDK matrix passes without forced peer
resolution.

## 4. Nix inputs, fixed-output hashes, toolchains, and CI actions

### Nix inputs

“Latest” is exact upstream head observed 2026-08-17; TASK-0107 must query again
and update each input separately.

| Scope/input | Declared channel | Locked rev (date) | Observed head | License/status | Advisory | Action | Source |
|---|---|---|---|---|---|---|---|
| root nixpkgs | GitHub default/master | `6bcaade2b819` (2026-08-13) | `4ab165655373` | MIT, active | source input; review upstream security feed | A/input / 0107 | [repo](https://github.com/NixOS/nixpkgs) |
| root flake-utils | default | `c1dfcf08411b` (2024-09-17) | `11707dc2f618` | MIT, active | none published | M/input / 0107 | [repo](https://github.com/numtide/flake-utils) |
| root nix-systems/default | transitive via utils | `da67096a3b9b` (2023-04-09) | same | MIT, maintained/current | none published | C | [repo](https://github.com/nix-systems/default) |
| pi nixpkgs | nixos-unstable | `2fcb964de67f` (2026-08-10) | `e5bdc4a41d4c` | MIT, active | source input; review upstream security feed | A/input / 0107 | [repo](https://github.com/NixOS/nixpkgs) |
| pi flake-utils | default | `11707dc2f618` (2024-11-13) | same | MIT, active | none published | C / 0107 | [repo](https://github.com/numtide/flake-utils) |
| pi nix-systems/default | transitive via utils | `da67096a3b9b` (2023-04-09) | same | MIT, maintained/current | none published | C | [repo](https://github.com/nix-systems/default) |

Root master and extension `nixos-unstable` remain distinct intentionally: root
packages Rust binaries; extension shell promises Node 24. Do not collapse them
without platform/toolchain proof.

Fixed-output Cargo `cargoHash` values are dependency identity and change only
after Cargo lock stabilization:

| Package expression | Current hash |
|---|---|
| `nix/package.nix` stable | `sha256-n9UHyr7W4hrN0+2dsYAYqkP/uzBv74p5XHU0g2MReJY=` |
| `nix/package-local.nix` | `sha256-n2QeEPsGo1/bnwpE8i+ttdGnAsuUa8qIrPJUpjqN8Zc=` |
| `nix/package-nightly.nix` | `sha256-m7qlL+ajw/rwIHQ7KAw7gI9QmpTBnxWEeTVRgrBOcl4=` |

### Toolchains and CI dependencies

| Dependency | Declared/current | Latest/required | License/status | Advisory | Action / owner | Source |
|---|---|---|---|---|---|---|
| Rust MSRV | `rust-version = 1.97`; CI/dev 1.97.1 | 1.97 minimum | MIT OR Apache-2.0, active | none known | pin/prove / 0108 | [Rust](https://www.rust-lang.org/tools/install) |
| Node engine | `>=22.19.0`; CI/flake Node 24 | Node 24 support gate; baseline 24.18.1 | MIT, active | npm audit green | retain / 0106-0108 | [Node](https://nodejs.org/) |
| Nix CLI | baseline 2.28.5 | flake-capable supported Nix | LGPL-2.1, active | none known | input/action refresh / 0107 | [Nix](https://github.com/NixOS/nix) |
| actions/checkout | root v3+v4; pi v4 | v7.0.1 | MIT, active | review release notes | A / 0108 | [action](https://github.com/actions/checkout) |
| actions/upload-artifact | v4 | v7.0.1 | MIT, active | review release notes | A / 0108 | [action](https://github.com/actions/upload-artifact) |
| actions/setup-node | pi v4 | v7.0.0 | MIT, active | review release notes | A / 0108 | [action](https://github.com/actions/setup-node) |
| cachix/install-nix-action | root v27 and v31 | v31.11.1 | Apache-2.0, active | review release notes | A / 0107 | [action](https://github.com/cachix/install-nix-action) |
| dtolnay/rust-toolchain | 1.97.1 | exact MSRV | MIT, active | review source ref | C / 0108 | [action](https://github.com/dtolnay/rust-toolchain) |
| softprops/action-gh-release | v2 | v3.0.2 | MIT, active | review release notes | A, release workflow / 0108 | [action](https://github.com/softprops/action-gh-release) |

Source actions are not covered by Cargo/npm advisory scanners. Major updates
require release-note/source review and immutable-ref policy in TASK-0108; no
silent major drift is accepted.

## 5. Immutable before-state

| Artifact | SHA-256 / identity |
|---|---|
| root commit at capture | `4233db1bd8a7797641800469700368590c8946d7` |
| pi-watcher gitlink/HEAD | `804da2f31d8e772c86eec344356a955985932d7e` |
| `Cargo.lock` | `95e369c842bf7ac9e8e89e8ab0061f082067d856e8e6d23def8d1ffe3b749345` |
| `pi-watcher/package-lock.json` | `ddf580bcb1d9c6df7ed361efe0616b9b4251a2a9a969b74dbb0f5f26a5e50457` |
| `flake.lock` | `b5466d76ce86de22396f67737eaa0522fb0598efdbe0d152ab38b6715aa0cfa6` |
| `pi-watcher/flake.lock` | `4def95521986389b78ae755341b793868b291c43a9844ab312a5487c04c78790` |

The submodule was clean at capture. Changes for TASK-0106 must land and verify in
that repository first; root then records one explicit gitlink commit. Never mix
uncommitted extension files with root dependency work.

## 6. Pre-upgrade baseline

| Gate | Result |
|---|---|
| Rust default full suite | pass (666 library tests plus integration binaries) |
| Rust full `test-integration` | watcher PASS gen=3, 376081ms |
| Focused process/reload/lifecycle | pass: 13 tests |
| Root format + unit + docs drift | watcher PASS gen=4/gen=5 |
| Root Nix flake + stable/nightly builds | watcher PASS gen=6 |
| pi-watcher `make all` | pass; coverage statements 92.97%, branches 87.26%, functions 95.59%, lines 94.70%; audit 0 |
| pi-watcher Node 24 Nix shell `make all` | pass on Node 24.18.1 |
| pi-watcher Nix flake evaluation | pass on aarch64-darwin; other systems omitted by local check |
| crate package list | pass, 230 files |
| npm package dry-run | pass, 21 files, 28,076 bytes packed / 107,110 unpacked |

Existing unrelated signal: first full integration attempt in TASK-0102 observed
one timing failure in `watcher_does_not_die_with_failing_tasks` (3 observations
vs expected 4); exact rerun and complete rerun passed. Dependency work must not
normalize or weaken that assertion.

## 7. Required success gates

An update family is acceptable only when relevant focused tests pass and final
chain proves all of these:

- Rust builds/tests with declared MSRV 1.97 (CI currently exact 1.97.1); no
  dependency silently raises it.
- pi-watcher passes `npm ci`, `make all`, package dry-run, and real-socket e2e
  under Node 24; Node 22 engine floor remains truthful if retained.
- `Cargo.lock` is deterministic under `cargo ... --locked`; clean `npm ci`
  reproduces `package-lock.json`; no unexpected lock diff.
- Reproducible scanner reports no unacknowledged vulnerable/yanked package and
  no high/critical advisory. Informational unmaintained yaml-rust remains an
  explicit TASK-0112 acknowledgement until replacement.
- `cargo tree --duplicates` and npm tree contain no unexplained duplicate major
  family or extraneous package.
- Root and extension flake checks pass; root stable/local/nightly builds and
  fixed-output hashes reproduce; extension Nix shell proves Node 24 gate.
- CLI help/schema/init, control wire snapshots, semantic config hashes,
  filesystem outcomes, process exits, and package file lists change only when
  explicitly approved and documented.

## 8. Ordered execution and rollback points

1. **TASK-0104 Rust low risk:** compatible runtime/dev refresh; serde derive
   consolidation; once_cell removal; SHA-2 separately. Commit each family.
2. **TASK-0105 filesystem checkpoint:** notify/debouncer adapter migration and
   duplicate removal; full filesystem matrix. Commit before process changes.
3. **TASK-0105 process checkpoint:** nix signal/process migration; full shutdown
   matrix. Separate commit from notify.
4. **TASK-0106 extension repository:** Pi pair together, then TypeBox/YAML, then
   compatible tooling. Commit and verify inside submodule; TypeScript 7 remains
   deferred unless ecosystem cap changes. Root gitlink is a separate commit.
5. **TASK-0107 packaging:** update each flake input explicitly; then recompute
   stable/local/nightly Cargo hashes after manifests settle.
6. **TASK-0108 final proof/automation:** advisory, MSRV, package, Nix, full
   integration/e2e, drift reporting, and grouped update automation.
7. **TASK-0112 parser replacement:** isolated from bulk chain; characterization
   tests first. It may proceed after 0103 but must not be folded into 0104/0105.

After every numbered checkpoint, preserve a green commit. Roll back with a new
`git revert <family-commit>` (never amend history), restore corresponding lock
and fixed-output hashes together, and rerun that checkpoint gate. For
pi-watcher, revert extension commit first and then commit root gitlink rollback.
This makes every dependency family bisectable and prevents cross-ecosystem
rollback coupling.
