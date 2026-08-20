---
id: TASK-0123
title: Prove and document user-approved job recoveries
status: doing
depends_on: [TASK-0122]
priority: high
tags: [integration-tests, docs, config, recovery, tty, reliability]
---

# Prove and document user-approved job recoveries

## Problem
Users and agents need end-to-end evidence and discoverable guidance showing when Funzzy offers a recovery, what is authorized, how verification works, and why a job ultimately passed or failed.

## Context

Primary example:

```yaml
execution:
  recovery_policy: prompt

jobs:
  - name: format-check @quick
    run: cargo fmt --all -- --check
    recovery: cargo fmt --all
```

Expected interaction after check failure:

```text
Job "format-check @quick" failed.
Proposed recovery:
  cargo fmt --all
Run this recovery and retry the job? [y/N]
```

## Acceptance criteria

- [ ] Add pseudo-TTY black-box tests proving prompt text, default decline, explicit approval, exact command execution, and one successful verification rerun for both `funzzy` and `fzz` binaries.
- [ ] Cover scalar and multi-command recoveries, recovery-command failure, verification failure, configured `skip`, `--recovery-policy skip` override, no-TTY behavior, EOF, cancellation, restart/supersession, and no second recovery attempt.
- [ ] Prove a formatting fixture changes only after approval and that declining leaves both workspace and original failure unchanged.
- [ ] Prove recoveries never overlap active parallel siblings or another recovery and are offered in job declaration order.
- [ ] Prove final exit code, watcher snapshot, failure list, retained output, structured events, fail-fast behavior, and generation success/failure hook agree with the final verified result.
- [ ] Prove valid hot reload adds/replaces/removes recoveries for later generations while an active generation retains its frozen commands and pending approval identity.
- [ ] Update canonical schema/examples/init output, README capability summary, USAGE, advanced watcher/control guidance, and relevant contracts using `recovery` consistently.
- [ ] Explain safety boundaries: config makes a recovery eligible, `prompt` needs attached-user approval, `y/N` defaults to no, `skip` never mutates, headless execution safely skips, and configured shell commands carry same trust model as `run`.
- [ ] Document CI usage with `--recovery-policy skip`, policy precedence, and why Funzzy does not infer approval from `CI=true` or ever auto-run recoveries in MVP.
- [ ] Explain difference between job `recovery` and generation `hooks.failure`, plus unsupported service jobs and MVP non-goals.
- [ ] Add a runnable formatting example and document exact behavior for `fzz run`, blocking watch, restart watch, and a watcher triggered through control socket.
- [ ] Run focused unit and integration targets through the watcher gate and capture proof that all acceptance criteria pass without flaky timing or external dependencies.

## Notes

Do not document automatic or remote acceptance until such a capability exists. Avoid examples that can destructively rewrite files outside an isolated fixture/workspace.
