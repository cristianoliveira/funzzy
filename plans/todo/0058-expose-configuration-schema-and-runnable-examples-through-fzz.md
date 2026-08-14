---
id: TASK-0058
title: Expose configuration schema and runnable examples through fzz
status: todo
depends_on: [TASK-0057, TASK-0005]
priority: high
tags: [rust, cli, axi, config, json-schema, examples, tdd]
---

# Expose configuration schema and runnable examples through fzz

## Problem
A written contract still requires agents to search files; Funzzy needs bounded, non-interactive commands that describe configuration and print valid examples from the installed binary.

## Context

Add real Clap `config` subcommands with local help and examples. Commands must work with no `.watch.yaml`, watcher, socket, network, or subprocess.

## Acceptance criteria

- [ ] Black-box tests first cover full/section schema, each example profile, unknown section/profile, clean stdout/stderr, no config present, and deterministic repeated output.
- [ ] `fzz config schema` emits valid deterministic JSON Schema for preferred grouped `.watch.yaml` format.
- [ ] `--section` returns bounded self-contained schema plus section identity and command hint for full schema.
- [ ] `fzz config example minimal|parallel|agent` emits valid runnable YAML to stdout with no prose mixed into document.
- [ ] Every emitted example parses through same production parser and passes available structural validation.
- [ ] Agent profile includes named target/tag, control socket, bounded concurrency, matching/ignore, fail-fast guidance where representable, and comments with next commands.
- [ ] Per-command `--help` states output contract, defaults, exit codes, and 2–3 copyable examples.
- [ ] Unknown input exits 2, names offending value, and lists exact valid alternatives without reading project configuration.
- [ ] Schema documents all accepted preferred keys and flags legacy/deprecated forms; no parser-accepted preferred key is omitted.
- [ ] Schema/spec output is versioned and additive compatibility policy is documented.
- [ ] Generated docs/optional agent guide use same source and stale-content CI test rather than copied handwritten field lists.

## Notes

