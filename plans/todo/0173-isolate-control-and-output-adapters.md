---
id: TASK-0173
title: Isolate control protocol and output adapters from domain
status: todo
depends_on: [TASK-0171, TASK-0172]
priority: normal
tags: [architecture, control, output, ports, protocol]
---

# Isolate control protocol and output adapters from domain

## Problem

`src/control.rs` and `src/output.rs` combine JSON-RPC/socket concerns with generation lookup, paging, retention, and domain outcome presentation. This couples domain evidence to a transport and makes protocol changes spread through execution code.

## Desired outcome

Expose typed domain queries and output pages through ports. Keep JSON-RPC parsing, Unix-socket transport, CLI formatting, stdout/logging, and Pi-watcher compatibility at the edge.

## Acceptance criteria

- [ ] Introduce typed internal requests/results for status, await, output, and cancellation without leaking JSON-RPC or socket types into domain modules.
- [ ] Keep paging, truncation, freshness, generation identity, and terminal outcome semantics unchanged.
- [ ] Prove domain query/result tests run without sockets, CLI, filesystem, process execution, stdout/logging, or watcher runtime.
- [ ] Preserve wire fields, error codes, formats, control-client behavior, and Pi-watcher compatibility.
- [ ] Add an import/dependency check proving adapters depend on domain ports and not the reverse.

## Verification

Run control/output unit tests, protocol and CLI tests, retained-output/cursor tests, Pi-watcher integration tests, full feature-gated integration, `make lint`, and all AST scans. Compare module centrality and quality findings with the baseline.
