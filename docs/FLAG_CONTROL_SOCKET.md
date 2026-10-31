# Control a running watcher through a Unix socket

Set `on.socket` in `.watch.yaml` to let clients inspect status and run named jobs. You can use `--control-socket PATH` to override that path for one invocation. A control socket implies `--on-busy restart`: a newer change can cancel obsolete work.

Prefer one shared project configuration:

```yaml
on:
  socket: .tmp/funzzy/control.sock

jobs:
  - name: final checks @agent-final
    run: cargo test
    change: ["src/**", "tests/**"]
```

Then start Funzzy without repeating the socket path:

```bash
fzz --log-file .tmp/funzzy/tests.log
```

`--control-socket PATH` takes precedence over `on.socket`. Funzzy creates missing parent directories and gives the socket `0600` permissions. It removes the socket on graceful shutdown. A live socket prevents a second server from using the same path; Funzzy replaces a stale socket file.

The socket uses JSON-RPC 2.0 framed as NDJSON: send one newline-terminated JSON-RPC request or batch per connection. Funzzy returns one newline-terminated response; notifications do not receive responses.

### Status

```json
{"jsonrpc":"2.0","id":"status","method":"status"}
```

```json
{"jsonrpc":"2.0","id":"status","result":{"generation":4,"state":"passed","trigger":"src/main.rs","commands":["cargo test"],"durationMs":42,"tasks":[{"id":"checks#1","name":"test","state":"passed","durationMs":37}],"failures":[]}}
```

A snapshot reports the current state, such as `idle`, `running`, `passed`, `failed`, or `cancelled`. A terminal snapshot also lists one `tasks` row per configured job, in declaration order. Each row's `durationMs` measures that job's elapsed time, whether jobs ran in series or parallel. `null` means the job never started or was skipped. A cancelled job that started keeps its partial duration.

Generation `durationMs` is separate wall-clock time, not a sum of job durations. Human `fzz control status` and `await` show `-` for an absent duration. Use JSON or TOON when you need integer milliseconds.

### List targets

```json
{"jsonrpc":"2.0","id":"targets","method":"targets"}
```

```json
{"jsonrpc":"2.0","id":"targets","result":[{"name":"final checks @agent-final","commands":["cargo test"]}]}
```

### Run named target

The target selects a named job in the running watcher:

```json
{"jsonrpc":"2.0","id":"run","method":"run","params":{"target":"@agent-final"}}
```

```json
{"jsonrpc":"2.0","id":"run","result":{"runId":5}}
```

Use `fzz ctl await --generation 5 --timeout 5m` to wait for that exact run. Do not infer its result by polling latest status: a newer run can supersede it. The [advanced control guide](ADVANCED-GUIDE.md#2-control-socket) explains exact-generation waits and output retrieval.

The socket response stays compact. Funzzy still writes full command output to the console and to `--log-file` when configured.
