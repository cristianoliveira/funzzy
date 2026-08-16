---
id: TASK-0096
title: Define complete removal boundary for docs drift apparatus
status: todo
depends_on: [TASK-0069, TASK-0095]
priority: high
tags: [design, docs, cleanup, ci, determinism]
---

# Define complete removal boundary for docs drift apparatus

## Problem
The docs-drift mechanism is a bad nondeterministic meta-gate that conditionally reuses binaries, launches nested builds/tests, scans prose with shell regexes, and couples documentation to release/workspace state; we want deletion, not redesign.

## Context

Scope selected explicitly: remove docs-drift only. Keep `scripts/version-check`, `scripts/version-check-test`, Nix bump helpers, and Git hook helper.

## Acceptance criteria

- [ ] Removal inventory names `scripts/docs-drift-check`, `make docs-drift`, push/release workflow calls and labels, `scripts/golden/init-template.yaml`, and tests/docs whose only purpose is generic docs-drift enforcement.
- [ ] No replacement umbrella script, generated-doc checker, internal-link checker, stale-vocabulary scanner, or docs-drift CI gate is proposed.
- [ ] Delete conditional stale `target/debug/fzz` reuse, nested Cargo invocations, Markdown/prose regex scans, temporary generated-file comparison, version-check delegation, and aggregated `DRIFT:` diagnostics as one apparatus.
- [ ] Product behavior tests remain when they verify runtime behavior directly (config parser accepts/rejects fields, `fzz init` creates runnable config, examples used by product); byte-for-byte documentation/golden policing is removed.
- [ ] Distinguish docs-only test from product contract test file-by-file before deletion; rationale is recorded for ambiguous `jobs_migration`, `command_init`, catalog parity, and init proof cases.
- [ ] `version-check` and release tag/Cargo/Cargo.lock identity checks remain independently owned and never become part of this removal.
- [ ] No change to generated `.watch.yaml` production content is included; removal only stops generic drift policing.
- [ ] Historical completed plan/report records may mention TASK-0069 but are not executable references and need not be rewritten.
- [ ] Current user/contributor docs must stop promising a docs-drift gate or `make docs-drift` command.
- [ ] Exact deletion list and retained safety checks are reviewed before implementation.

## Notes

This task intentionally chooses deletion over making docs-drift deterministic.
