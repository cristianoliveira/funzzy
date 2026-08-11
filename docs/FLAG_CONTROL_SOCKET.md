## FLAG: `--control-socket <path>`

Expose compact watcher state and named-target execution over a Unix domain socket. This mode implies `--non-block` so a newer filesystem event or control request can cancel an obsolete run.

```bash
mkdir -p .tmp/funzzy
fzz --control-socket .tmp/funzzy/control.sock \
  --log-file .tmp/funzzy/tests.log
```

The parent directory must already exist. Funzzy creates the socket with permissions `0600` and removes it on graceful shutdown. A live socket at the same path prevents a second server from starting; stale socket files are replaced.

The protocol is versioned NDJSON: one JSON request and one JSON response per connection.

### Status

```json
{"v":1,"id":"status","method":"status"}
```

```json
{"v":1,"id":"status","result":{"generation":4,"state":"passed","trigger":"src/main.rs","commands":["cargo test"],"durationMs":42,"failures":[]}}
```

States are `idle`, `running`, `passed`, `failed`, and `cancelled`.

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
