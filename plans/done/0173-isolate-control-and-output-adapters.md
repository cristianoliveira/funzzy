---
id: TASK-0173
title: Isolate control protocol and output adapters from domain
status: done
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

- [x] Introduce typed internal requests/results for status, await, output, and cancellation without leaking JSON-RPC or socket types into domain modules.
- [x] Keep paging, truncation, freshness, generation identity, and terminal outcome semantics unchanged.
- [x] Prove domain query/result tests run without sockets, CLI, filesystem, process execution, stdout/logging, or watcher runtime.
- [x] Preserve wire fields, error codes, formats, control-client behavior, and Pi-watcher compatibility.
- [x] Add an import/dependency check proving adapters depend on domain ports and not the reverse.

## Verification

Run control/output unit tests, protocol and CLI tests, retained-output/cursor tests, Pi-watcher integration tests, full feature-gated integration, `make lint`, and all AST scans. Compare module centrality and quality findings with the baseline.

## Evidence

- Typed seams: output gained `RetrievalFields`/`RetrievalRequest`/`RetrievalMode`/`RetrievalStream`/`RequestError` with pure validation (`e72e254`, `99a0cb8`); status (`WatcherState`+`FailureEvidence`/`StatusSnapshot`), await (`AwaitParams`/`AwaitSnapshot`), and cancel (`CancelResult`) were already typed. JSON-RPC/Unix-socket shaping remains only in `control.rs`.
- Wire compatibility: `control.rs` maps typed errors to byte-identical payloads (-32602/-32012/-32013/-32014/-32010/-32011/-32015 unchanged); control lib tests 61 passed including wire-shape cases; feature-gated `control_output` (11) and `control_cancel` (6) passed sequentially; no payload changed, so no pi-watcher coordination was required (submodule untouched).
- Pure tests: 7 `output::request_tests` validate mode/tail/full/cursor exclusivity and budget clamping with no JSON, sockets, CLI, filesystem, process, or watcher types; full output suite 35 passed in-process.
- Import direction: module graph shows no domain→infrastructure edge (cycles unchanged: cli/config, executor/stdout); `tests/domain_boundaries.rs` 8 passed; inventory documented in `docs/DOMAIN-BOUNDARIES.md` (`1fa2715`).
- Gates: `make lint` and `cargo fmt -- --check` clean; watcher gen152 full unit gate passed fresh; watcher gen153 integration gate passed on unchanged fingerprint `9ae1c747ce47`.
- Scans: control.rs top-5 complexity average 1.8 and output.rs 3.6, zero high-complexity functions; SOLID findings recorded (control 4, output pre-existing adapter scope); no centrality regression (top fan-in unchanged: plan 16, executor 15, rules 15).
- Known issue (out of scope, pre-existing on baseline): `tests/control_await.rs` service-marker races under parallel threads (`service_pid` line 251, `wait_until` timeouts); sequential runs pass.

