# Target selection (V2)

Replaces the removed V1 `--target` flag. In V2, target selection is a
positional argument on the relevant subcommand.

## Selecting targets

```sh
fzz watch "@quick"      # watch only matching targets
fzz run "@quick"        # run once, locally
fzz list                # show every configured target
fzz explain PATH        # which targets a path matches or is ignored by
```

A target is a job name, `@tag`, or an unambiguous substring. `@tag` may select
many jobs; a plain substring must be unambiguous or it is an error listing
the alternatives.

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

`TARGET` is selected first. Each `--exclude TARGET` then resolves against the
full configured job set: an exact name wins; `@tag` excludes every tagged job;
a plain substring must identify one job. Missing or ambiguous selectors, and
an effective plan with no runnable jobs, are usage errors (exit 2). Repeated or
overlapping exclusions are harmless. `--no-services` excludes all jobs with
`service: true`, including readiness-enabled services, before watcher roots or
process lifecycle setup.

Filters are per invocation. They are not persisted, do not affect `fzz run` or
control requests, and remain active when the watcher reloads its configuration.

See docs/USAGE.md §2 for the decision table and the normative
CLI-V2-CONTRACT for the command tree.
