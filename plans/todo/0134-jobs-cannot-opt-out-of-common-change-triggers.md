---
id: TASK-0134
title: Jobs cannot opt out of common change triggers
status: todo
depends_on: []
priority: normal
tags: [rust, config, parser, jobs-contract]
---

# Jobs cannot opt out of common change triggers

## Problem

`on.change` common triggers are merged into every job's `change:`
(`merge_patterns`, `src/config.rs:431`), documented as an invariant:
"Tasks EXTEND the shared `on` rules; they never replace them... root
level scope and safety rails always apply". A job cannot declare "I
react only to my own triggers" while sibling jobs keep inheriting the
common ones. Today's only escape is leaving `on.change` empty and
duplicating the common patterns into every job that wants them.

Concrete consequences:

1. **Finite jobs**: a docs or deploy job fires on every `src/**` edit
   just because the config shares common triggers.
2. **Services**: with common triggers merged, a `service: true` job is
   re-included — hence restarted (restart-by-reinclusion model,
   TASK-0133) — on every common event. Each restart kills in-flight
   work and resets cadence; a 60s poller under continuous editing
   effectively never completes a poll.

Compositional caveat (26-08-26 probe, `.tmp/reports/26-08-26/gh-actions-watch-design-a.md`):
excluded from the superseding plan does not mean "left alone" for a
service — it is killed by the generation replacement (`src/workers.rs`
consumer loop -> `executor.cancel` reaps `run.services`). Any opt-out
mechanism must account for that current semantics.

## How to reproduce (motivation)

```yaml
on:
  change: ["src/**"]

jobs:
  - name: deploy docs
    change: ["docs/**"]
```

Touch `src/lib.rs`: the deploy job runs although its declared trigger
never matched.

## Acceptance criteria

- [ ] A job can be configured so common `on.change` triggers do not
  apply to it, while sibling jobs keep inheriting them.
- [ ] Opting out is explicit and validated; ambiguous or partially
  specified shapes are rejected with an actionable error, consistent
  with existing strictness (unknown job properties, JOBS-CONFIG-
  CONTRACT §5).
- [ ] Existing merge semantics are unchanged for configs that do not
  opt out (backward compatible, no behavior change on reload).
- [ ] The chosen shape is reflected consistently across `fzz check`,
  `fzz config`, `fzz init` catalog, and `fzz explain` (explain shows
  the effective pattern set for the job).
- [ ] The config revision hash distinguishes configs that differ only
  in a job's trigger inheritance, so hot reload reacts to the change.
- [ ] Preferred `jobs:` form only; legacy grouped `tasks:` configs
  keep merge-only behavior (compatibility surface).
- [ ] Decision recorded for `ignore` interaction: whether job `ignore`
  may also be overridden or always merges, with the tradeoff stated.
- [ ] Documented interaction with `service: true` under the current
  restart-by-reinclusion model, including the kill-on-supersede caveat
  above.
- [ ] Tests beside `config.rs` for parse/validate/reload-hash, plus
  one integration test in `tests/`: an opted-out job does not fire on
  a common trigger while a sibling merged job does.

## Scope

- `src/config.rs`, `src/option_catalog.rs`, `src/config_revision.rs`
- Docs: USAGE / ADVANCED-GUIDE / JOBS-CONFIG-CONTRACT
- Tests beside modules; integration coverage in `tests/`
