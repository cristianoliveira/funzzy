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

See docs/USAGE.md §2 for the decision table and the normative
CLI-V2-CONTRACT for the command tree.
