### Examples

Here you find the shipped workflow catalog used by Funzzy documentation and
integration tests. The migration target for valid examples is the canonical
`on`/`execution`/`hooks`/`jobs` vocabulary. The complete inventory, approved
filename map, and before/after behavior contract are in
[`docs/EXAMPLES-V2-MIGRATION-CONTRACT.md`](../docs/EXAMPLES-V2-MIGRATION-CONTRACT.md).

## Changed-path inputs

Choose the smallest input that fits the script:

- Use `FZZ_TRIGGER_PATH` for one trigger path.
- Use `FZZ_TRIGGER_RELATIVE_PATH` for that path relative to the job's `cwd`.
- Use `FZZ_TRIGGER_PATHS_FILE` for the complete changed-path batch.

The batch variable points to a private UTF-8 JSON array of literal paths. Read
it while Funzzy supervises the child; do not save the pointer for use after the
job exits. Init and manual runs are pathless and receive no pointer. Hooks do
not receive one implicitly. See
[Changed-path inputs](../docs/USAGE.md#changed-path-inputs) for the full
lifecycle contract.

### Runnable batch scripts

The Node and Python examples have the same behavior. Each reads the JSON array
immediately and prints one JSON-quoted path per line, so spaces, quotes, and
newlines remain data rather than shell syntax.

| Example | Prerequisite | Workflow | Script |
| --- | --- | --- | --- |
| Node | `node --version` (Node 18 or newer) | [`node/workflow.yml`](changed-path-batch/node/workflow.yml) | [`print-changed-paths.mjs`](changed-path-batch/node/print-changed-paths.mjs) |
| Python | `python3 --version` (Python 3.8 or newer) | [`python/workflow.yml`](changed-path-batch/python/workflow.yml) | [`print_changed_paths.py`](changed-path-batch/python/print_changed_paths.py) |

These are optional examples, not Funzzy dependencies. The language runtime is
enough for the direct manifest demonstrations below. Install `fzz` only to run
the live watcher workflows.

#### Node workflow

The workflow assumes its example directory is the working directory. From the
repository root, start it with:

```bash
cd examples/changed-path-batch/node
node --version                 # v18 or newer
fzz --version
mkdir -p input
fzz -c workflow.yml watch
```

In another terminal, from the same directory, create one rapid multi-file
batch. This includes spaces, a quote, and a newline in filenames:

```bash
node - <<'NODE'
const fs = require("node:fs");
for (const path of [
  "input/one.txt",
  "input/two words.txt",
  "input/quote\"name.txt",
  "input/line\nbreak.txt",
]) fs.writeFileSync(path, "changed\n");
NODE
```

The watcher prints all paths as JSON strings. A manual run is pathless and
prints `No changed-path batch (init or manual run).`:

```bash
fzz -c workflow.yml run "print complete changed-path batch"
```

To exercise only the script, without installing Funzzy:

```bash
manifest=$(mktemp)
printf '%s\n' '["input/one.txt","input/two words.txt","input/quote\"name.txt","input/line\nbreak.txt"]' > "$manifest"
FZZ_TRIGGER_PATHS_FILE="$manifest" node ./print-changed-paths.mjs
rm -f "$manifest"
```

The observable batch output is:

```text
"input/one.txt"
"input/two words.txt"
"input/quote\"name.txt"
"input/line\nbreak.txt"
```

#### Python workflow

Start the Python workflow from its own directory:

```bash
cd examples/changed-path-batch/python
python3 --version              # 3.8 or newer
fzz --version
mkdir -p input
fzz -c workflow.yml watch
```

In another terminal, from the same directory, create the batch:

```bash
python3 - <<'PY'
from pathlib import Path

for path in [
    "input/one.txt",
    "input/two words.txt",
    'input/quote"name.txt',
    "input/line\nbreak.txt",
]:
    Path(path).write_text("changed\n", encoding="utf-8")
PY
```

Run the same pathless manual check with:

```bash
fzz -c workflow.yml run "print complete changed-path batch"
```

Or exercise only the script:

```bash
manifest=$(mktemp)
printf '%s\n' '["input/one.txt","input/two words.txt","input/quote\"name.txt","input/line\nbreak.txt"]' > "$manifest"
FZZ_TRIGGER_PATHS_FILE="$manifest" python3 ./print_changed_paths.py
rm -f "$manifest"
```

This prints the same four JSON-quoted paths shown in the Node example.

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
