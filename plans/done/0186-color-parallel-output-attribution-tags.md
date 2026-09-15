---
id: TASK-0186
title: Color parallel output attribution tags
status: done
depends_on: [TASK-0028, TASK-0041]
priority: normal
tags: [rust, output, parallel, ux, tdd]
---

# Color parallel output attribution tags

## Problem
Parallel task output is line-atomic and attributed as [job], but identical plain tags are hard to scan when concurrent jobs interleave. Users need automatic, readable visual attribution without changing child output, logs, captured evidence, or machine-readable control responses.

## Context

`src/cmd.rs` renders live parallel lines as `[label] body`; `src/executor.rs`
also reveals failed buffered output as `[label:stdout|stderr] body`. Both routes
must use one presentation formatter. The existing `FUNZZY_COLORED` environment
gate controls terminal color; logs already strip ANSI and retained output is raw.
Do not add YAML configuration or override child-process environment: terminal
color follows the existing gate, including `_TEST_FUNZZY_COLORED` in
feature-gated integration tests.

Color only the bracketed attribution tag. Select its color deterministically
from its label using a fixed terminal-safe palette. This gives one label the
same color across stdout, stderr, partial lines, repeated commands, and failure
reveal without depending on timing or completion order. Palette collisions are
acceptable; identity remains the literal tag.

## Acceptance criteria

- [x] With `FUNZZY_COLORED` enabled, every live line from a named parallel job
  colors only its `[job]` tag; its child-produced body, newline, raw capture,
  task identity, and line-atomic forwarding stay unchanged.
- [x] The tag color is deterministic from the label, and the same label has the
  same color for stdout, stderr, complete lines, partial final lines, and later
  commands, regardless of parallel completion order.
- [x] `show-on-failure` uses the same formatter for `[job:stdout]` and
  `[job:stderr]`; the stream suffix remains visible and buffered output still
  appears exactly once.
- [x] With color disabled, live and failure-reveal output remains byte-for-byte
  compatible with the current plain `[label] body` forms. The implementation
  honors the existing `FUNZZY_COLORED` gate (and its feature-gated
  `_TEST_FUNZZY_COLORED` test counterpart), without adding configuration.
- [x] ANSI affects terminal presentation only: `--log-file` stays plain,
  retained output stays raw, diagnostics and structured control/event output
  contain no ANSI, and no config, protocol, selector, signature, or snapshot
  field changes.
- [x] Unit tests cover the pure color selection/tag formatter and disabled
  behavior. Black-box tests cover two interleaved parallel labels plus stdout,
  stderr, a partial line, and one failure reveal with colors enabled; assertions
  are task-keyed rather than completion-order dependent.

## Delivery plan

1. Add the pure deterministic label-to-palette/tag renderer at the output
   presentation edge (`src/stdout.rs` or a focused presentation module),
   gated only by the existing color environment policy.
2. Route `cmd::render_live_line` and `executor::reveal_on_failure` through it;
   keep raw capture, logger writes, and control adapters outside the formatter.
3. Add focused unit and `tests/run_once.rs` integration coverage, then run the
   focused commands and a fresh watcher gate.

## Ownership and verification

- Implementation owner: Dave (dev).
- Independent verifier: Kely (qa).

## Verification

```sh
cargo test --lib cmd::tests::render_live_line
cargo test --lib executor::tests::parallel_group_tasks_get_live_output_labels_serial_tasks_do_not
cargo test --test run_once parallel_live_output_is_attributed_and_summary_names_group_and_tasks
cargo test --test run_once
```

Then run the configured watcher final gate on the unchanged worktree.

## Evidence

- `a92d0f3` introduced the shared formatter and both attribution paths;
  `f47a462` completed black-box coverage.
- Kely independently passed the focused colored run, feature-gated colored
  run, and all 20 `run_once` tests.
- Watcher generation 15 passed `cargo fmt --all -- --check && cargo test`.

## Notes

- Keep standard diagnostic colors (`Funzzy`, `Error`, `Success`) out of scope.
- Do not promise a unique color per arbitrary label: a finite palette can
  collide. The literal label remains the authoritative identity.
