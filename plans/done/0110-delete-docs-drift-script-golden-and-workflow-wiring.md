---
id: TASK-0110
title: Delete docs drift script golden and workflow wiring
status: done
depends_on: [TASK-0109]
priority: high
tags: [docs, cleanup, ci, makefile, github-actions]
---

# Delete docs drift script golden and workflow wiring

## Problem
Keeping fragments after deleting the umbrella script would leave misleading Make targets CI names golden snapshots tests and documentation claims that imply the bad gate still exists.

## Context

Remove all live entry points in one change so no command or CI label points to missing behavior.

## Acceptance criteria

- [ ] Delete `scripts/docs-drift-check` and `scripts/golden/init-template.yaml`; remove empty `scripts/golden` directory.
- [ ] Delete `docs-drift` target and `.PHONY` declaration from Makefile.
- [ ] Remove Docs/CLI/schema/example drift step from `.github/workflows/on-push.yml` without replacement.
- [ ] Remove `make docs-drift` from release workflow and rename combined “Version + docs drift” step to describe retained exact version/tag identity only.
- [ ] Remove direct byte-golden init test and docs-only size/snapshot assertions that depend on deleted golden; retain behavioral `fzz init` create/check/run tests.
- [ ] Remove broad examples/doc-block drift tests only when TASK-0109 classifies them as docs policing; retain focused migration/parser behavior tests.
- [ ] Remove current README/docs/Make help references that advertise generic docs drift prevention or command, while keeping installed schema/init documentation truthful.
- [ ] Do not alter `scripts/version-check`, `scripts/version-check-test`, their Make targets, or release identity shell assertions.
- [ ] Do not add replacement shell/Python/Rust meta-check or snapshot framework.
- [ ] Production source and generated init output remain byte-identical unless compilation requires deleting dead test-only exposure.

## Notes
Renumbered from duplicate TASK-0096/0097/0098 (collided with init/config stream from f1e08e2) to restore unique IDs; deps updated in kind.
