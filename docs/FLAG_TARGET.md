# Target selection (V2)

Use `fzz watch TARGET` to watch selected jobs or `fzz run TARGET` to run them once. V2 removed the V1 `--target` flag.

## Selecting targets

```sh
fzz watch "@quick"      # watch only matching targets
fzz -- "@quick"         # equivalent watch shorthand; target follows `--`
fzz run "@quick"        # run once, locally
fzz list                # show every configured target
fzz explain PATH        # which targets a path matches or is ignored by
```

`fzz -- TARGET` is an alias for `fzz watch TARGET`. The root form requires `--`. Without a target, `fzz --` watches all configured jobs. Put root options before the delimiter, for example `fzz --exclude lint -- @quick`.

Funzzy treats the value after `--` as target data, even if it looks like a subcommand or starts with `-`. More than one target is a usage error. Without the delimiter, subcommands keep their usual meaning.

`watch TARGET` and `fzz -- TARGET` select every job whose name contains the target text. Multiple jobs can match one substring. An `@tag` selects every job with that tag.

`run TARGET` is stricter. An exact job name wins; an `@tag` can still select several jobs. Any other substring must identify exactly one job, or Funzzy reports the alternatives.

## Migration from V1

| V1 | V2 |
| --- | --- |
| `fzz --target <text>` | `fzz watch <text>` / `fzz run <text>` |
| `fzz -t <text>` | same — `-t` is removed |
| (list targets) | `fzz list` |

Behavior change: `watch TARGET` with no match is an actionable error (not a
silent all-tasks fallback); `run TARGET` selects the exact target and rejects
path arguments. Exit-code impact: no-match/ambiguous is exit 1 with the
available-targets list.

## Per-invocation watch exclusions

Watch-only filters compose with target selection without editing the config:

```sh
fzz watch --exclude lint
fzz watch "@quick" --exclude docs --exclude "slow check"
fzz watch --no-services
```

Funzzy selects `TARGET` first, using the watch rules above. It then resolves each `--exclude TARGET` against all configured jobs. An exact name wins. An `@tag` excludes all jobs with that tag. Another substring must identify one job.

A missing or ambiguous selector, or a plan with no runnable jobs, exits 2. Repeated or overlapping exclusions do no harm. `--no-services` excludes every `service: true` job, including services with readiness checks. Funzzy applies these filters before it sets up watcher roots or processes.

Filters are per invocation. They are not persisted, do not affect `fzz run` or
control requests, and remain active when the watcher reloads its configuration.

See docs/USAGE.md §2 for the decision table and the normative
CLI-V2-CONTRACT for the command tree.
