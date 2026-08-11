## FLAG: `--control-socket <path>`

Expose compact watcher state and named-target execution over a Unix domain socket. This mode implies `--non-block` so a newer filesystem event or control request can cancel an obsolete run.

Prefer one shared project configuration in `.watch.yaml`:

```yaml
control:
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

`--control-socket <path>` remains an explicit override and takes precedence over `.watch.yaml`. Configuring either form implies `--non-block`.

Funzzy creates missing parent directories and creates the socket with permissions `0600`. It removes the socket on graceful shutdown. A live socket at the same path prevents a second server from starting; stale socket files are replaced.

The protocol is versioned NDJSON: one JSON request and one JSON response per connection.

### Status

```json
{"v":1,"id":"status","method":"status"}
```

```json
{"v":1,"id":"status","result":{"generation":4,"state":"passed","trigger":"src/main.rs","commands":["cargo test"],"durationMs":42,"failures":[]}}
```

States are `idle`, `running`, `passed`, `failed`, and `cancelled`.

### List targets

```json
{"v":1,"id":"targets","method":"targets"}
```

```json
{"v":1,"id":"targets","result":[{"name":"final checks @agent-final","commands":["cargo test"]}]}
```

### Run named target

The target uses the same task-name substring matching as `--target`:

```json
{"v":1,"id":"run","method":"run","params":{"target":"@agent-final"}}
```

```json
{"v":1,"id":"run","result":{"runId":5}}
```

Poll `status` until `generation` equals `runId` and state is terminal. If generation becomes greater than `runId`, that run was superseded.

Full command output remains on stdout and in `--log-file`; socket responses intentionally stay compact for agents and editor integrations.
