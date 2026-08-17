---
id: TASK-0112
title: Replace unmaintained yaml-rust parser with maintained implementation
status: todo
depends_on: [TASK-0103]
priority: high
tags: [rust, yaml, security, config, migration, tdd]
---

# Replace unmaintained yaml-rust parser with maintained implementation

## Problem
yaml-rust 0.4.5 is unmaintained under RUSTSEC-2024-0320; replacing a configuration parser can change accepted YAML, diagnostics, ordering, and migration bytes, so it requires isolated compatibility proof rather than a bulk dependency update.

## Context

RustSec recommends the maintained `yaml-rust2` fork. Preserve the V2 config,
migration, schema, comments, quoting, ordering, and diagnostic compatibility
surfaces. This task is deliberately separate from TASK-0104/TASK-0105 so a
parser behavior change remains independently reviewable and reversible.

Sources:

- <https://rustsec.org/advisories/RUSTSEC-2024-0320.html>
- <https://github.com/Ethiraric/yaml-rust2>

## Acceptance criteria

- [ ] Characterization tests cover preferred `jobs:`, legacy root-list/grouped forms, aliases, multiline commands, comments, quoting, nulls, duplicate keys, malformed YAML, and migration byte stability before changing parser.
- [ ] Evaluate maintained alternatives for license, MSRV 1.97, API fit, maintenance, and known advisories; record why selected implementation wins.
- [ ] Replace `yaml-rust` without broad dependency updates or unrelated config refactors.
- [ ] Accepted/rejected configuration matrix and user-facing error categories remain compatible, or intentional breaking differences are documented before implementation acceptance.
- [ ] `fzz migrate` remains atomic/idempotent and preserves comments, quotes, ordering, commands, and newline behavior proved by existing black-box tests.
- [ ] Remove `yaml-rust` and close acknowledgement of RUSTSEC-2024-0320; advisory scan contains no unacknowledged result.
- [ ] Unit, config workflow, migration, reload, docs drift, integration, and Nix package gates pass with no unexplained lockfile changes.

## Notes

Do not hide parser behavior differences behind permissive fallback parsing.

