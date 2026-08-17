---
id: TASK-0109
title: Define complete removal boundary for docs drift apparatus
status: done
depends_on: [TASK-0069, TASK-0095]
priority: high
tags: [design, docs, cleanup, ci, determinism]
---

# Define complete removal boundary for docs drift apparatus

## Problem
The docs-drift mechanism is a bad nondeterministic meta-gate that conditionally reuses binaries, launches nested builds/tests, scans prose with shell regexes, and couples documentation to release/workspace state; we want deletion, not redesign.

## Context

Scope selected explicitly: remove docs-drift only. Keep `scripts/version-check`, `scripts/version-check-test`, Nix bump helpers, and Git hook helper.

## Removal boundary (complete inventory, TASK-0110 executes this)

### A. Delete wholesale

| Item | Path | What it is |
|---|---|---|
| Umbrella script | `scripts/docs-drift-check` | All 6 checks: delegated example/doc-block tests, version-check delegation, stale-vocabulary regex, markdown link scan, golden snapshot + size budget with conditional `target/debug/fzz` reuse and nested `cargo build` |
| Make target | `Makefile:108-110` | `.PHONY docs-drift` + rule |
| Push CI step | `.github/workflows/on-push.yml:33-34` | "Docs/CLI/schema/example drift check" → `make docs-drift` |
| Release CI call | `.github/workflows/on-release.yml:46` | `make docs-drift` line inside identity gate (identity checks lines 35-45 REMAIN) |
| Golden bytes | `scripts/golden/init-template.yaml` | Byte-for-byte snapshot of init output (+ then-empty `scripts/golden/` dir) |

### B. Surgical edits

| File | Change |
|---|---|
| `.watch.yaml` | Task "unit and drift checks" → rename "unit tests"; drop `make docs-drift` run line; drop `scripts/docs-drift-check` from its `change:` list. (Repo's own watcher config — not generated template content.) |
| `docs/INIT-TEMPLATE-CONTRACT.md:153` | Strike "exact bytes are frozen by the golden snapshot … drift gate fails" sentence; ceiling stays normative, enforcement removed. |
| `docs/V2-DOCS-ARCHITECTURE.md` | Lines 14-21 "TASK-0069 drift gate" column → "review"; line 70 "(TASK-0069 enforces no drift)" → drop parenthetical; line 95 tree "TASK-0069 drift CI" → drop node. |
| `docs/RELEASE-BOUNDARY.md:76` | Drop "requires TASK-0069 (docs drift CI)" row dependency. |
| `README.md:200` | Keep "runnable example" claim, drop "— docs never drift" trailer. |
| `tests/command_init.rs` | Keep file-creation + success-message assertions; delete the golden-equality assert block (lines ~40-52) — that is byte-policing. |

### C. Test classification (docs-only vs product, file-by-file)

**`tests/jobs_migration.rs`** (8 tests):
- DELETE `usage_guide_yaml_blocks_parse_through_the_production_parser` (l.225) — parses Markdown prose blocks from docs/USAGE.md, ADVANCED-GUIDE.md, RUN-HOOKS-CONTRACT.md. Pure docs-policing; no product fixture involved.
- KEEP `every_example_passes_fzz_check` (l.259) — runs the real `fzz check` binary over shipped `examples/*.yml` product fixtures. Direct runtime verification that shipped examples remain valid configs (matches retention criterion "examples used by product").
- KEEP remaining 6 — identical list/barrier output, idempotent migrate, parallel semantics + sequential override, mixed-input rejection, cwd/env/init run parity. All product behavior.

**`tests/command_init.rs` / `command_init_errors.rs`**: KEEP all (creation, message, refusal, read-only errors) minus the golden assert noted above.

**`tests/command_init_proof.rs`** (TASK-0095 proof) — KEEP all 5; rationale per case:
- `init_is_deterministic_single_file_and_refuses_overwrite` — self-referential determinism (two runs compared), no golden dependency. Product.
- `generated_config_runs_init_and_change_jobs` — template is runnable end-to-end. Product.
- `documented_values_parse_and_invalid_alternatives_fail` — parser accept/reject matrix. Product.
- `template_comments_cover_catalog_without_unsupported_properties` — **catalog parity, ambiguous**: compares product-generated output against the product's own `option_catalog` in src/. Catches real generator bugs (catalog gains a property, renderer drops it). Internal product consistency, not prose policing. KEEP.
- `example_profiles_stay_lean_and_do_not_inherit_init_output` — verifies `fzz config example` runtime output. Product.

### D. Retained safety checks (independently owned — NOT part of removal)

- `scripts/version-check` + `make version-check` (TASK-0061 identity).
- `scripts/version-check-test`.
- on-release.yml tag/Cargo/Cargo.lock identity gate (lines 35-45, separate from the deleted call).
- `make prebuild/lint/tests`, integration suite, Nix gates.
- Unit tests co-located with parser/catalog/init renderer.

### E. Non-goals (explicit)

- No replacement umbrella script, generated-doc checker, internal-link checker, stale-vocabulary scanner, or docs-drift CI gate.
- No change to generated `.watch.yaml` template content (product code `src/cli/init.rs` untouched).
- Historical plans/reports may mention TASK-0069; not rewritten.

## Acceptance criteria

- [x] Removal inventory names `scripts/docs-drift-check`, `make docs-drift`, push/release workflow calls and labels, `scripts/golden/init-template.yaml`, and tests/docs whose only purpose is generic docs-drift enforcement.
- [x] No replacement umbrella script, generated-doc checker, internal-link checker, stale-vocabulary scanner, or docs-drift CI gate is proposed.
- [x] Delete conditional stale `target/debug/fzz` reuse, nested Cargo invocations, Markdown/prose regex scans, temporary generated-file comparison, version-check delegation, and aggregated `DRIFT:` diagnostics as one apparatus.
- [x] Product behavior tests remain when they verify runtime behavior directly; byte-for-byte documentation/golden policing is removed.
- [x] Distinguish docs-only test from product contract test file-by-file; rationale recorded for `jobs_migration`, `command_init`, catalog parity, and init proof cases.
- [x] `version-check` and release tag/Cargo/Cargo.lock identity checks remain independently owned and never become part of this removal.
- [x] No change to generated `.watch.yaml` production content; removal only stops generic drift policing.
- [x] Historical completed plan/report records may mention TASK-0069 but need not be rewritten.
- [x] Current user/contributor docs stop promising a docs-drift gate or `make docs-drift` command (section B).
- [x] Exact deletion list and retained safety checks reviewed before implementation (TASK-0110 executes this list verbatim).

## Notes

This task intentionally chooses deletion over making docs-drift deterministic.
Renumbered from duplicate TASK-0096/0097/0098 (collided with init/config stream from f1e08e2) to restore unique IDs; deps updated in kind.
