---
id: TASK-0016
title: Make ad-hoc exec mode preserve child argv
status: done
depends_on: [TASK-0015]
priority: high
tags: [rust, cli, exec, stdin, tdd]
---

# Make ad-hoc exec mode preserve child argv

## Problem
The current arbitrary-command mode accepts one opaque string, forcing quoting and obscuring the boundary between Funzzy options and child arguments.

## Context

Use `fzz exec -- PROGRAM ARG...`. Shell operators only work when caller explicitly invokes a shell.

## Acceptance criteria

- [x] Tests first cover argv preservation, child flags, missing command, stdin paths, no stdin, non-zero child exit, and shell-explicit execution.
- [x] Child program and arguments cross parser/runtime boundaries without being joined and reparsed.
- [x] `--` gives an unambiguous boundary between Funzzy and child options.
- [x] Existing path templates receive same relative and absolute path values.
- [x] Command startup and failure errors identify whether Funzzy or child process failed.
- [x] Behavior is proven through spawned-binary integration tests.

## Notes

