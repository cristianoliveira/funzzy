---
id: TASK-0069
title: Prevent documentation CLI schema and example drift in CI
status: todo
depends_on: [TASK-0068]
priority: high
tags: [docs, ci, drift, links, examples, determinism]
---

# Prevent documentation CLI schema and example drift in CI

## Problem
A one-time rewrite will decay unless command help, generated schema, checked examples, links, versions, and migration vocabulary are continuously verified against executable behavior.

## Context

Prefer focused deterministic checks over screenshots or external-link dependence in every push. External URL checks may run scheduled with bounded retries.

## Acceptance criteria

- [ ] CI compares captured `funzzy`/`fzz` command trees and local subcommand help to reviewed golden/generated reference.
- [ ] Config schema, generated examples, and generated doc fragments have one stale-content check that fails with exact regeneration command.
- [ ] All YAML examples parse/check; shell command smoke tests execute in isolated workspaces without starting unbounded watchers.
- [ ] Internal markdown links/anchors and referenced repository paths are validated on push.
- [ ] External links are checked on bounded scheduled job with allowlist and actionable report, not flaky required push gate.
- [ ] Stale V1 vocabulary/version scan excludes explicit migration/contract contexts and reports file/line.
- [ ] Code blocks declare language and runnable blocks are identified deterministically rather than guessed.
- [ ] Release CI proves docs version/status, Cargo/tag, stable Nix, capabilities fixture, and install commands agree.
- [ ] Documentation gate runtime/output remains bounded and focused command exists for local repair.

## Notes

