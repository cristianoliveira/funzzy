## FLAG: `--control-socket <path>`

Expose compact watcher state and named-target execution over a Unix domain socket. This mode implies `--non-block` so a newer filesystem event or control request can cancel an obsolete run.

Prefer one shared project configuration in `.watch.yaml`:

```yaml
on:
  socket: .tmp/funzzy/control.sock

tasks:
  - name: final checks @agent-final
    run: cargo test
    change: ["src/**", "tests/**"]
```

Then start Funzzy without repeating the socket path:

```bash
fzz --log-file .tmp/funzzy/tests.log
```

`--control-socket <path>` remains an explicit override and takes precedence over `on.socket` in `.watch.yaml`. Configuring either form implies `--non-block`.

Funzzy creates missing parent directories and creates the socket with permissions `0600`. It removes the socket on graceful shutdown. A live socket at the same path prevents a second server from starting; stale socket files are replaced.

The socket uses JSON-RPC 2.0 framed as NDJSON: send one newline-terminated JSON-RPC request or batch per connection. Funzzy returns one newline-terminated response; notifications do not receive responses.

### Status

```json
{"jsonrpc":"2.0","id":"status","method":"status"}
```

```json
{"jsonrpc":"2.0","id":"status","result":{"generation":4,"state":"passed","trigger":"src/main.rs","commands":["cargo test"],"durationMs":42,"failures":[]}}
```

States are `idle`, `running`, `passed`, `failed`, and `cancelled`.

### List targets

```json
{"jsonrpc":"2.0","id":"targets","method":"targets"}
```

```json
{"jsonrpc":"2.0","id":"targets","result":[{"name":"final checks @agent-final","commands":["cargo test"]}]}
```

### Run named target

The target uses the same task-name substring matching as `--target`:

```json
{"jsonrpc":"2.0","id":"run","method":"run","params":{"target":"@agent-final"}}
```

```json
{"jsonrpc":"2.0","id":"run","result":{"runId":5}}
```

Poll `status` until `generation` equals `runId` and state is terminal. If generation becomes greater than `runId`, that run was superseded.

Full command output remains on stdout and in `--log-file`; socket responses intentionally stay compact for agents and editor integrations.
