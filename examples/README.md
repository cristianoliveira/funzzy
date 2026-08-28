### Examples

Here you find the shipped workflow catalog used by Funzzy documentation and
integration tests. The migration target for valid examples is the canonical
`on`/`execution`/`hooks`/`jobs` vocabulary. The complete inventory, approved
filename map, and before/after behavior contract are in
[`docs/EXAMPLES-V2-MIGRATION-CONTRACT.md`](../docs/EXAMPLES-V2-MIGRATION-CONTRACT.md).

To inspect or run one of these examples from the repository root:

```bash
fzz -c examples/simple-case.yml check
fzz -c examples/simple-case.yml list
fzz -c examples/simple-case.yml watch
```

Example: Run `fzz -c examples/simple-case.yml` and then change files in `examples/workdir/` to check the output.

Those examples are all valid and used in [the integration tests](https://github.com/cristianoliveira/funzzy/tree/main/tests).

- `recovery-format.yml` demonstrates a bounded, explicitly approved formatting
  recovery. Use `--recovery-policy skip` for headless or CI execution.
