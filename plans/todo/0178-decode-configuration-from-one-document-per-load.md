---
id: TASK-0178
title: Decode configuration from one document per load
status: doing
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

## Pre-move criterion-to-evidence matrix and load inventory

| Criterion | Current owner/path | Pre-move evidence or gap |
| --- | --- | --- |
| One read/parse per aggregate load | `reload_session::run` reads once, then `control_socket_from_yaml`; `reload::decide`/`validate_candidate` reparses each field; `build_watches_from_content` reparses again. `app::watch_action` calls `load_rules` plus independent policy accessors, each reopening the file. | Gap confirmed: reload and startup perform repeated parses/reads. |
| Preferred/legacy/grouped and discovery | `config::from_yaml`/`from_file` plus `from_default_file_config`; `.watch.yaml` then `.watch.yml` fallback. | Existing config parser and migration tests characterize accepted shapes. |
| Missing vs explicit values | Individual `*_from_yaml` accessors apply defaults independently (`Option`/default arguments). | Existing accessor tests; same-document aggregate must preserve distinction. |
| Error precedence/messages | `reload::validate_candidate` parses rules, validates, then policy accessors in fixed order; startup calls accessors in action-specific order. | Existing reload/config error tests; preserve first-error order. |
| Same text for rules/runtime | Reload has one `content` string but repeatedly reparses it; startup repeatedly rereads the path. | Structural gap; no behavior test can prove read count today. |
| Pure validation boundary | `config_validation` is used by parser/rules validation; no filesystem there. | Existing domain-boundary tests; preserve imports. |
| Revision/freezing/socket handoff | `reload_session` → `reload::decide` → `ReloadCoordinator`; startup freezes `RuntimeConfig` then reload commits candidate revision/socket. | Existing reload identity/lifecycle/socket tests. |

Actual read/parse inventory (baseline):

- `reload_session` performs one `read_to_string` per changed candidate, then parses socket once, `reload::validate_candidate` parses rules plus each runtime accessor, and `build_watches_from_content` parses rules plus each runtime accessor again.
- `app::watch_action` first parses rules, then independently reads debounce/backend/gitignore/recovery policy/timeout/hooks/session hooks, socket, and later rereads several fields while capturing the initial revision.
- `app::check_config` reads rules and then rereads hooks, session hooks, debounce, recovery policy, and concurrency.
- `config::*_from_file` accessors are compatibility wrappers that each open/read and delegate to a `*_from_yaml` parser; `from_default_file_config` preserves `.yaml` then `.yml` fallback and first-error behavior.

Pre-move characterization commands/results are recorded in `.tmp/reports/04-09-26/task-0178-discovery.md` before implementation.

## First seam evidence

- Inventory and criterion status: `.tmp/reports/04-09-26/task-0178-first-seam.md`.
- Commit `3757b9b` introduces private `ConfigDocument`: one `YamlLoader` result supplies rules and all runtime policy readers. Existing YAML/file accessors remain compatibility wrappers.
- Reload now performs one candidate read and one parse per attempt, carries the resulting `RuntimeConfig` through revision observation, and builds `Watches` without reparsing. Rules parse remains syntactic; later policy errors remain semantic and ordered.
- Pre-move config/reload/migration characterization passed: config 126, reload 16, config reload lifecycle 14, reload matrix 7, invalid reload 2, config workflow 6, migration 6. Fresh watcher gen205 passed.
- Task remains `doing`: startup `watch_action` and `check_config` still call independent file accessors, so aggregate one-read/one-parse is only partial. No closure until those paths are migrated or a follow-up boundary is explicitly accepted.

## Non-goals

No YAML-library migration, schema redesign, changed defaults, changed semantic hashing, or replacement of `RuntimeConfig`.
