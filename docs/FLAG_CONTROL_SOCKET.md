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

### Remote control over SSH (for agents)

No Funzzy server change is needed. Run the watcher on the remote host with `on.socket` configured, and use OpenSSH to forward that Unix socket to the machine where the agent runs `fzz ctl`. The SSH command runs **on the agent's machine**, not on the watcher host. Replace `user@host` and `/absolute/path/to/remote/.watch.sock` with the SSH target and the **absolute path of the running watcher's socket** (a relative `on.socket` resolves from the watcher's working directory).

```sh
# On the agent's machine; keep this directory private.
mkdir -p "$HOME/.local/state/funzzy"
chmod 700 "$HOME/.local/state/funzzy"

# In a terminal that stays open while the agent uses fzz ctl:
ssh -N -o ExitOnForwardFailure=yes \
  -L "$HOME/.local/state/funzzy/box.sock:/absolute/path/to/remote/.watch.sock" \
  user@host
```

In another terminal **on that same agent machine**:

```sh
fzz ctl --socket "$HOME/.local/state/funzzy/box.sock" capabilities --format toon
fzz ctl --socket "$HOME/.local/state/funzzy/box.sock" status --format toon
fzz ctl --socket "$HOME/.local/state/funzzy/box.sock" list --format toon
```

To trigger a job, choose a target from `list`; for example, `fzz ctl --socket "$HOME/.local/state/funzzy/box.sock" run '@agent-final' --wait --timeout 5m --format toon`. Use the returned generation for subsequent `await` or `output` calls. Do not repeat a `run` after a connection drop without checking status: the remote watcher may already have accepted it.

**Requirements and limits:** SSH must authenticate to the intended host, and the SSH account must be allowed to connect to the remote socket. The SSH server must permit Unix-socket forwarding (`AllowStreamLocalForwarding`); no TCP control port is opened. Anyone who can connect to the forwarded local socket has that account's **full** Funzzy control authority, including `run` and `cancel`, so do not place it in a shared directory or share the SSH credentials. Keep the SSH command running while using the socket; stop it when finished. If `box.sock` already exists, check whether an older forward is still running before removing a stale socket. `ExitOnForwardFailure` catches a failed local bind, but a missing remote socket is reported only when `fzz ctl` connects. This recipe requires OpenSSH with Unix-domain socket forwarding on both ends; it is not a Windows portability promise.

For a project's `AGENTS.md`, record the SSH target, the absolute remote `on.socket` path, and this guide's link. Do **not** record SSH secrets. The project's agents can then create their own private local forward and use `fzz ctl --socket` without changing Funzzy configuration.
