# Dependency update policy and tooling (TASK-0108)

How dependencies are updated, reviewed, and rolled back across the four
ecosystems: Rust (Cargo), Node (pi-watcher npm), Nix (root + pi-watcher
flakes), and the pi-watcher submodule gitlink.

> Companion contracts: `docs/RELEASE-BOUNDARY.md` (version identity),
> `scripts/version-check` (release surfaces). Automation **proposes** updates;
> behavior-sensitive majors are always reviewed through a board task.

## Cadence

| Channel | What | When |
|---|---|---|
| `deps-drift.yml` scheduled workflow | cargo audit, npm audit/outdated, nix input freshness — grouped per ecosystem, informational | weekly (Mon 06:00 UTC) |
| Renovate (root + pi-watcher configs) | grouped update PRs: nix inputs (monthly), cargo lock maintenance (monthly), majors always separate | when installed |
| Manual (TDD task per major) | watcher/process crate majors, Pi SDK pair, TypeScript toolchain, Node major, Rust MSRV | per release planning |

Routine PR CI (`on-push.yml`) stays deterministic/offline: build, lint,
unit tests, MSRV job. Nothing in it reaches the network for dependencies.

## Manual update commands

### Rust (root)

```sh
cargo update -p <crate> --precise <version>   # targeted, reviewed
make lint && make tests                       # fast loop
fzz control run @agent-final --wait           # full + integration gates
nix run github:nixos/nixpkgs/937e5ee4bb456986f217f3b9bcfde35de4b7fb81#cargo-audit -- audit
cargo tree --duplicates                       # explain every duplicate
./scripts/bump-nix-local                      # refresh local cargoHash + version label
```

The `cargo-audit` command is pinned to the root flake's nixpkgs rev so the
scanner version is reproducible; refresh that pin when the flake input moves.

### Node (pi-watcher)

```sh
npm ci                                        # baseline reproduction
npm outdated && npm audit --audit-level=high  # survey
npm install <pkg>@<exact>                     # exact pins (policy: no ranges)
make quick && make all                        # 452 tests incl. real-socket e2e
npm pack --dry-run                            # package surface unchanged
```

Pi dev pins (`@earendil-works/pi-ai`,
`@earendil-works/pi-coding-agent`) move **together** to the same release.
`peerDependencies` stay broad; dev pins prove the exact supported integration.

### Nix

```sh
nix flake update nixpkgs            # root — explicit input, never blanket update
nix flake check && nix build .#local .#nightly .#default --no-link
cd pi-watcher && nix flake update nixpkgs
nix develop -c sh -c 'npm ci && make all'   # devshell proof, Node major preserved
```

Root and pi-watcher keep **distinct nixpkgs pins on purpose**: the root needs
a Rust toolchain for package builds; pi-watcher needs only the Node devshell.
Do not collapse them without a dedicated design task.

Fixed-output hashes are computed only through `scripts/bump-nix-local` /
`scripts/bump-nix-nightly` (never hand-guessed). The **stable** package
(`nix/package.nix`) moves only at release publication (TASK-0063 flow).

## Review checklist (before merging any dependency change)

1. Lockfile diff contains **only** intended version moves — no drive-by churn.
2. `cargo tree --duplicates` / `npm ls --all`: no new unexplained duplicate majors.
3. Advisory scans clean: `cargo audit` (pinned command above), `npm audit --audit-level=high`.
4. Gates green: `make lint`, full test suite, integration suite (watcher `@agent-final`),
   pi-watcher `make quick`/`make all` + `npm pack --dry-run`, `nix flake check`,
   package builds, `scripts/version-check --candidate`.
5. Behavior surfaces compared green: `tests/cli_arguments.rs` (CLI contract),
   `tests/control_*` (wire snapshots), `tests/command_init_proof.rs` (init/catalog),
   `src/config.rs` suite (parser accept/reject). Any intentional behavioral
   diff is documented in the task record before acceptance (cf. TASK-0112
   yaml-rust2 YAML 1.2 deltas).
6. MSRV: `Cargo.toml` `rust-version` unchanged unless the update forces it;
   if raised, the `msrv` CI job pin moves in the same change.
7. Root gitlink updated explicitly when pi-watcher commits land, with evidence.

## Rollback

- Revert the manifest+lockfile commit(s) (`git revert`), never amend history.
- Rust: reverting `Cargo.toml`+`Cargo.lock` restores the tree; re-run gates.
- pi-watcher: revert inside the submodule, then update the root gitlink with
  a revert commit of its own.
- Published crates.io versions are **never** silently fixed by yank:
  yank/un-yank is a separate incident decision (RELEASE-BOUNDARY).
