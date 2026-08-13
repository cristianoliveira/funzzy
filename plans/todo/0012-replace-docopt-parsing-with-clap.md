---
id: TASK-0012
title: Replace Docopt parsing with Clap
status: todo
depends_on: [TASK-0010, TASK-0011]
priority: high
tags: [rust, cli, refactor]
---

# Replace Docopt parsing with Clap

Issue: https://github.com/cristianoliveira/funzzy/issues/226

## Problem

Docopt is unmaintained. Current parser catches its missing-value error, mutates argv, and parses again to support `fzz --target` without a value. Parser-shaped fields (`flag_*`, empty strings, ignored command flags) leak into application dispatch.

## Deliverable

Clap-backed parser boundary and semantic application arguments, with Docopt and its workaround fully removed.

## Scope

- New `src/arguments.rs`
- `src/app.rs` argument consumption and dispatch
- `src/lib.rs` module wiring
- `Cargo.toml` and `Cargo.lock`
- Parser unit tests

## Acceptance criteria

- [ ] Current Clap 4.6 is used with explicitly configured short/long options.
- [ ] `docopt` is absent from manifest, lockfile, imports, and dependency tree.
- [ ] `src/arguments.rs` owns parser details and exposes semantic action/option types to application code.
- [ ] Value-less `-t/--target` is represented directly as list-targets mode without error interception, argv mutation, or second parse.
- [ ] `-v/--version` and `-V` preserve Funzzy meanings.
- [ ] `init`, `watch`, configured watch, and quoted arbitrary-command forms preserve characterized behavior.
- [ ] Parser design does not use external-subcommand capture that swallows supported global options after command.
- [ ] Dynamic `GITSHA` version suffix is preserved.
- [ ] Application dispatch uses named semantic fields and early returns/matches instead of Docopt-shaped flags and empty-string sentinels.
- [ ] Config discovery, stdin handling, target filtering, environment overrides, logging, and watch strategy behavior remain unchanged.
- [ ] `serde`/`serde_derive` remain available for control protocol serialization.

## Verification

- Parser unit tests cover happy and unhappy paths.
- `cargo test --test cli_arguments`
- Existing target, init, config, logging, stdin, fail-fast, non-block, and control-socket tests pass.
- `cargo tree` and repository search show no Docopt.
