---
id: TASK-0021
title: Add CLI client for the control socket
status: doing
depends_on: [TASK-0015, TASK-0042]
priority: high
tags: [rust, cli, ipc, control-socket, json-rpc, tdd]
---

# Add CLI client for the control socket

## Problem
The Unix socket is currently usable only through raw JSON-RPC or the Pi extension, so terminal users cannot discover status, targets, or trigger a running Funzzy process through the normal CLI.

## Context

Provide a client command group for the existing JSON-RPC methods rather than requiring users to construct NDJSON manually:

```text
fzz control status
fzz control list
fzz control run TARGET
```

Resolve socket path from explicit `--socket`, then configured `on.socket`. This task consumes existing `status`, `targets`, and `run` methods; it does not invent synthetic filesystem events.

## Acceptance criteria

- [ ] Tests first cover status, list, run scheduling, unavailable socket, malformed response, request timeout, and server error.
- [ ] `control` has real nested Clap subcommands and command-specific help.
- [ ] Client resolves explicit socket override before project configuration and reports selected path on connection failure.
- [ ] `status` renders current generation, state, trigger, duration, commands, and failures concisely.
- [ ] `list` renders remote targets from running watcher rather than reparsing local configuration.
- [ ] `run TARGET` returns scheduled generation identity; atomic `await` and `run --wait` are added by TASK-0044.
- [ ] JSON-RPC framing, request IDs, error objects, and response validation live in a client adapter, not CLI presentation.
- [ ] Socket communication has bounded connect/read/wait timeouts and deterministic exit codes.
- [ ] Raw command output remains owned by watcher stdout/log file; client output stays compact.

## Notes

TASK-0022 adds `control emit PATH` as a separate protocol/routing change. This task remains limited to existing methods so client architecture is proven before protocol expansion.

