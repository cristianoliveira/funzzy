---
id: TASK-0066
title: Rewrite V2 getting started configuration and daily workflows
status: todo
depends_on: [TASK-0065, TASK-0058, TASK-0033]
priority: high
tags: [docs, readme, configuration, workflows, onboarding, v2]
---

# Rewrite V2 getting started configuration and daily workflows

## Problem
README and usage material do not provide one short accurate path from installation to configuring, checking, listing, explaining, running, and watching current V2 workflows.

## Context

Use installed self-description as configuration reference. Handwritten docs teach decisions and workflow; they should link/call `fzz config schema`, `fzz config example`, and `fzz check` instead of maintaining another field catalog.

## Acceptance criteria

- [ ] README starts with one-sentence identity, supported V2 status, minimal install, runnable configured workflow, `fzz run`, zero-argument/watch behavior, and next-doc links.
- [ ] Getting-started path works in clean temp directory and reaches first successful finite target before introducing control socket or advanced features.
- [ ] Configuration guide explains preferred grouped shape, matching/ignore precedence, templates, tags/targets, cwd/env, init behavior, parallel groups/barriers/concurrency, and config discovery/check commands.
- [ ] Daily-workflow guide distinguishes `fzz`, `watch`, local `run`, ad-hoc argv-preserving `exec`, `list`, `explain`, and control commands with decision table.
- [ ] Wait/restart, fail-fast, output/logging, exit codes, and common recovery actions use current names and tested behavior.
- [ ] Installation covers Cargo, GitHub binaries, and Nix without claiming unpublished/stale versions.
- [ ] Every copyable command and YAML block is executed/parsed by tests or generated from checked examples.
- [ ] Content is concise, cross-linked, and avoids repeating normative parser/schema lists.
- [ ] Both executable names are explained once; examples prefer `fzz` consistently.

## Notes

