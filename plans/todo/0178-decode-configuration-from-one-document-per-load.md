---
id: TASK-0178
title: Decode configuration from one document per load
status: todo
depends_on: []
priority: high
tags: [rust, config, architecture, readability]
---

# Decode configuration from one document per load

## Problem
Startup and reload assemble configuration through independent YAML and file readers. Setting precedence and error ordering are distributed, and repeated file reads can observe different contents.

## Context

`src/reload.rs::validate_candidate` calls independent parsers for rules and runtime settings; each reparses YAML. `src/app.rs` also uses independent file readers. TASK-0170 already separated pure validation into `config_validation`; this task addresses document loading and assembly, not that completed validation extraction.

## Scope and approach

1. Characterize preferred/legacy documents, missing versus explicit fields, startup/reload defaults, and first-error ordering.
2. Add a private parsed-document owner and root-based field readers. Keep existing string/file accessors as compatibility wrappers.
3. Route reload through one parsed document, retaining field evaluation order. Then route each startup/check load through one read and decode boundary.
4. Keep decoded input separate from effective `RuntimeConfig`; preserve startup and reload policy differences explicitly rather than unifying them by accident.
5. Give the default config filename a configuration owner instead of importing `cli::watch`; retain the old constant path as a compatibility alias if necessary.

## Acceptance criteria

- [ ] Each aggregate load attempt reads the selected file once and parses its text once; a later reload is a separate attempt.
- [ ] Existing preferred jobs, legacy lists, grouped/nested formats, and `.yaml`/`.yml` discovery behavior remain accepted as before.
- [ ] Missing and explicitly declared values remain distinguishable until defaults are applied.
- [ ] Error categories, precedence, and public messages remain unchanged; cover invalid YAML and competing field errors.
- [ ] Rules and runtime settings for one load derive from the same input text.
- [ ] `config_validation` stays pure; no CLI, watcher, or filesystem imports enter domain policy.
- [ ] Reload revision identity, active-generation freezing, socket handoff, and invalid-candidate shutdown behavior remain unchanged.

## Verification

Run focused config/config-validation/reload/app tests and feature-enabled config-check, reload, and migration coverage. Verify one-read/one-parse structure through the load seam, not timing benchmarks. Use the fresh watcher final gate and document before/after dependency direction.

## Likely files

`src/config.rs` or private config submodules, `src/app.rs`, `src/reload.rs`, `src/cli/watch.rs` for a compatibility alias, and focused tests.

## Non-goals

No YAML-library migration, schema redesign, changed defaults, changed semantic hashing, or replacement of `RuntimeConfig`.
