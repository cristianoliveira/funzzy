---
id: TASK-0135
title: Define explicit manual-only job trigger contract
status: todo
depends_on: [TASK-0134]
priority: high
tags: [design, config, jobs, manual, control-socket, determinism]
---

# Define explicit manual-only job trigger contract

## Problem

Jobs intended only for explicit local or control invocation currently require `change` or `run_on_init`. Root `on.change` is also merged into every job. Users must therefore accept accidental execution, disable init globally, duplicate configuration, or invent an impossible path glob.

This blocks a clear integration-agnostic command-observation recipe: the blocking script should start only after the user explicitly requests its target.

## Preferred API to evaluate

```yaml
jobs:
  - name: await-remote
    trigger: manual
    run: ./scripts/await-remote.sh
```

`trigger: manual` is the working proposal, not permission to broaden this task into a generic event-source or plugin model. If evidence requires another name or shape, record the tradeoff and preserve the behavior below.

## Acceptance criteria

- [ ] Define manual as explicit invocation through `fzz run TARGET` or `fzz ctl run TARGET`; it never means provider polling, webhook intake, or arbitrary control-socket command execution.
- [ ] Define that a manual-only job does not inherit root `on.change`, does not match filesystem events, and does not run at watcher initialization.
- [ ] Define validation for combinations with `change`, `run_on_init`, and `service`; reject ambiguous combinations with actionable errors rather than invent precedence silently.
- [ ] Define selection behavior for exact names, substrings, and tags across local and control runs.
- [ ] Define `fzz list`, `fzz explain PATH`, schema, example, and help presentation so users can discover that the job is manual-only.
- [ ] Preserve existing merge semantics and runtime behavior for every configuration without the new explicit shape.
- [ ] Limit the preferred shape to `jobs:`; legacy task-list and grouped `tasks:` compatibility remains unchanged.
- [ ] Include the trigger mode in semantic config revision identity so hot reload cannot retain stale behavior.
- [ ] Define reload behavior for a currently running manual finite job using existing frozen-generation semantics.
- [ ] Record security boundary: control clients may select only configured targets and cannot provide arbitrary commands.
- [ ] Record non-goals and implementation/test impact in a dedicated contract or existing configuration contract.

## Non-goals

- General opt-out/replace semantics for non-manual change-triggered jobs.
- Per-invocation arguments, environment injection, or secrets over the control socket.
- Execution timeouts (TASK-0138 through TASK-0140).
- Provider adapters or structured script results.
- Changes to `service: true`.
