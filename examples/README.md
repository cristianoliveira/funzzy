### Examples

Here you find the shipped workflow catalog used by Funzzy documentation and
integration tests. The migration target for valid examples is the canonical
`on`/`execution`/`hooks`/`jobs` vocabulary. The complete inventory, approved
filename map, and before/after behavior contract are in
[`docs/EXAMPLES-V2-MIGRATION-CONTRACT.md`](../docs/EXAMPLES-V2-MIGRATION-CONTRACT.md).

## Changed-path inputs

New job scripts should read `FZZ_TRIGGER_PATH` for one trigger file,
`FZZ_TRIGGER_RELATIVE_PATH` for that file relative to the job's `cwd`, or the
`FZZ_TRIGGER_PATHS_FILE` JSON array for the complete batch. These values exist
only in filesystem-triggered job children. Init and manual runs are pathless,
hooks receive none of them, and the batch pointer is valid only while Funzzy
supervises the job or service. JSON entries are literal paths, not shell-escaped
command text. See the [current migration examples](../docs/USAGE.md#changed-path-inputs).

`jobs-with-filepath-template.yml` and `jobs-with-absolute-paths.yml` remain in
the catalog as compatibility examples. Path templates—including
`{{filepath}}`, `{{absolute_path}}`, `{{relative_filepath}}`,
`{{relative_path}}`, `{{paths}}`, and spacing variants—still execute as before,
but are deprecated for new job scripts. In particular, `{{paths}}` remains one
expanded element in direct-argv commands. Funzzy does not warn or rewrite these
legacy commands.

To inspect or run one of these examples from the repository root:

```bash
fzz -c examples/simple-case.yml check
fzz -c examples/simple-case.yml list
fzz -c examples/simple-case.yml watch
```

Example: Run `fzz -c examples/simple-case.yml` and then change files in `examples/workdir/` to check the output.

Those runnable examples are used in [the integration tests](https://github.com/cristianoliveira/funzzy/tree/main/tests).
The files under `examples/invalid/` are intentionally invalid V2 fixtures:
use them with `check` to inspect validation errors, but do not run them as
workflows. The recursive catalog gate checks both `.yml` and `.yaml` files and
keeps these invalid fixtures separate from runnable examples.

- `recovery-format.yml` demonstrates a bounded, explicitly approved formatting
  recovery. Use `--recovery-policy skip` for headless or CI execution.
