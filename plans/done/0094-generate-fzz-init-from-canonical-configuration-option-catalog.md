---
id: TASK-0094
title: Generate fzz init from canonical configuration option catalog
status: done
depends_on: [TASK-0093, TASK-0058, TASK-0069]
priority: high
tags: [rust, cli, init, config, schema, tdd]
---

# Generate fzz init from canonical configuration option catalog

## Problem
A hand-maintained comprehensive template would immediately duplicate parser and schema knowledge; one canonical supported-field catalog must drive the commented init reference and installed schema.

## Context

Create typed metadata for supported YAML fields instead of adding another unrelated string constant. Rendering can contain layout groups, but property identity/default/type/enum/help must have one owner.

## Acceptance criteria

- [ ] Tests first expose current drift: accepted hooks/service/output fields missing from schema/template and any schema pseudo-fields that are not legal YAML properties.
- [ ] Canonical option catalog covers `on.change`, `ignore`, `socket`, `concurrency`, `debounce`, `watch_backend`, `poll_interval`, `respect_gitignore`, `success`, `failure`, `output`; and job `name`, `run`, `change`, `ignore`, `run_on_init`, `parallel`, `cwd`, `env`, `service`, `output` plus any supported fields found by TASK-0093 inventory.
- [ ] Catalog records owner/path, required/default, accepted type/enum, short explanation, and rendering example without secret material.
- [ ] `fzz config schema` and comprehensive commented init renderer consume same property metadata; parser allowlists/error messages consume it or parity tests enforce exact equivalence where parser refactor is unsafe.
- [ ] Active generic starter remains explicit: hello `echo` with `run_on_init`, plus harmless `ls` file-change example and control socket as currently useful.
- [ ] Optional entries render commented and uncommenting documented scalar examples produces parser-valid values.
- [ ] Array/string command and glob forms and template variables are demonstrated compactly without activating duplicate jobs.
- [ ] `InitCommand` preserves create-only behavior, custom filename behavior, deterministic bytes, and `--migrate` semantics.
- [ ] Schema exposes only legal preferred YAML fields in structural definitions; conceptual CLI/protocol sections remain clearly separate and cannot masquerade as config keys.
- [ ] Renderer uses plain YAML comments and requires no network/repository inspection.

## Notes
