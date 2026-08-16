---
id: TASK-0095
title: Prove init template completeness readability and drift safety
status: done
depends_on: [TASK-0094, TASK-0059, TASK-0069]
priority: high
tags: [integration-tests, cli, init, config, schema, docs, reliability]
---

# Prove init template completeness readability and drift safety

## Problem
Unit rendering alone cannot prove a newly created .watch.yaml is deterministic runnable accepted by production check and complete for every supported global and job field.

## Context

Exercise installed binary in empty temporary workspace. Assertions inspect behavior and catalog coverage, not fragile prose wholesale except one reviewed golden snapshot.

## Acceptance criteria

- [ ] Black-box test proves `fzz init` creates exactly one deterministic `.watch.yaml`, `fzz check` accepts it, and second init refuses overwrite without mutation.
- [ ] Starting generated config runs generic init example successfully with no Cargo/npm/language dependency.
- [ ] Creating matching generic file after startup triggers generic change example, proving generated starter can be tried immediately.
- [ ] Catalog parity test fails when parser-supported preferred property is absent from schema or init comments, and when template/schema advertises unsupported property.
- [ ] Each documented enum/default/example is accepted by production parser; invalid alternatives still fail deterministically.
- [ ] Golden snapshot reviews section order, brief explanations, all-commented optional settings, active example size, commands, quoting, and bounded total bytes/lines.
- [ ] `fzz init custom.yml`, existing-file protection, read-only/error path, and `fzz init --migrate` remain unchanged.
- [ ] `fzz config example minimal|parallel|agent` stay runnable and do not inherit human-commented init output accidentally.
- [ ] README/getting-started describes comprehensive generated reference and immediate `fzz init && fzz` trial accurately.
- [ ] Documentation/schema/example drift CI includes catalog and init snapshot and passes from installed version-matched binary.

## Notes
