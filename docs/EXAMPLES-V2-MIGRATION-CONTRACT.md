# Shipped examples V2 migration contract

**Status:** TASK-0144 design contract. This document defines the migration
boundary for the checked-in `examples/` catalog; it does not rewrite examples
or change the production parser.

The catalog is both user-facing documentation and an integration-test fixture.
A migration is correct only when the preferred V2 configuration preserves the
observable behavior of the accepted legacy configuration. Legacy parser
compatibility remains supported and is tested separately.

## Scope and inventory

The recursive catalog contains 17 YAML configurations: 14 valid public or
integration examples and 3 intentionally invalid fixtures. `examples/workdir/`
files and `examples/longtask.sh` are runtime fixtures, not configurations.

| File | Classification | Current shape | TASK-0146 action |
| --- | --- | --- | --- |
| `common-rules.yml` | Direct migration: grouped root | root `on` + root `tasks` | rename `tasks` to `jobs`; retain root policy |
| `list-of-failing-commands.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `list-of-tasks-run-on-init.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `nested-groups.yml` | Semantic migration: nested groups | root list of `on`/`tasks` groups | flatten groups into ordered `jobs` |
| `recovery-format.yml` | Already V2 | `execution` + `jobs` | byte-identical no-op |
| `reload-config-example.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename not required |
| `simple-case.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; retain file |
| `tasks-with-absolute-paths.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `tasks-with-filepath-template.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `tasks-with-long-running-commands.yaml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `tasks-with-tags-to-filter.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `test-nested-groups.yml` | Semantic migration: nested groups + regular job | mixed root list | flatten groups, preserve regular-job position |
| `v2-parallel-control.yml` | Already V2 | canonical `on`/`execution`/`jobs` | byte-identical no-op |
| `workflow-with-dependency-between-tasks.yml` | Direct migration: root list | ordered root task list | wrap as ordered `jobs`; rename file |
| `invalid/invalid-value.yml` | Intentionally invalid V2 fixture | malformed glob scalar | reshape root to `jobs`, retain YAML parse failure |
| `invalid/missing-required-property.yml` | Intentionally invalid V2 fixture | job item without `name` | reshape root to `jobs`, retain missing-name failure |
| `invalid/non-list.yaml` | Intentionally invalid V2 fixture | `on` has the wrong type | retain V2 `jobs`, retain `on` object-type failure |

Classification totals: 2 already V2, 1 directly migratable grouped root, 9
directly migratable root lists, 2 nested shapes requiring semantic flattening,
and 3 intentionally invalid fixtures.

## Filename migration map

The following is the complete public filename policy. A `same` mapping is
explicit: it prevents ad-hoc aliases and makes the no-rename decision
reviewable. TASK-0146 updates active tests, docs, README links, fixture
constants, and reload append logic atomically. Historical reports and
completed-plan evidence may retain the old spelling as historical text.

| Current filename | V2 filename | Reason |
| --- | --- | --- |
| `common-rules.yml` | `common-rules.yml` | domain-neutral name |
| `list-of-failing-commands.yml` | `jobs-with-failing-commands.yml` | remove root-list/task teaching surface |
| `list-of-tasks-run-on-init.yml` | `jobs-run-on-init.yml` | use V2 job vocabulary |
| `nested-groups.yml` | `nested-job-groups.yml` | make grouped job vocabulary explicit |
| `recovery-format.yml` | `recovery-format.yml` | already V2 and referenced publicly |
| `reload-config-example.yml` | `reload-config-example.yml` | domain-neutral name |
| `simple-case.yml` | `simple-case.yml` | domain-neutral name |
| `tasks-with-absolute-paths.yml` | `jobs-with-absolute-paths.yml` | use V2 job vocabulary |
| `tasks-with-filepath-template.yml` | `jobs-with-filepath-template.yml` | use V2 job vocabulary |
| `tasks-with-long-running-commands.yaml` | `jobs-with-long-running-commands.yaml` | use V2 job vocabulary; retain `.yaml` coverage |
| `tasks-with-tags-to-filter.yml` | `jobs-with-tags-to-filter.yml` | use V2 job vocabulary |
| `test-nested-groups.yml` | `test-nested-job-groups.yml` | test fixture names the V2 subject |
| `v2-parallel-control.yml` | `v2-parallel-control.yml` | already canonical |
| `workflow-with-dependency-between-tasks.yml` | `workflow-with-job-dependencies.yml` | use V2 job vocabulary |

No compatibility symlink or duplicate legacy filename is allowed. Parser
compatibility belongs in dedicated parser/migration fixtures, not duplicate
public examples.

## Transformation and flattening semantics

### Direct forms

1. A root list is wrapped under one `jobs:` key. Every item remains an ordered
   job item. Commands, quoting, comments, names, tags, `run_on_init`, `change`,
   `ignore`, and all other accepted job fields are preserved.
2. A grouped root `{on: ..., tasks: [...]}` changes only the root key to
   `jobs:`. Root policy text stays in place; no policy field is relocated to
   `execution` or `hooks` by this migration.
3. An already-V2 file is a byte-identical migration no-op. Migration must not
   normalize whitespace, quote style, comments, or line endings.

### Nested groups

Nested groups cannot be represented as objects inside a V2 `jobs:` list. They
are flattened in one global declaration order:

- Visit groups and ordinary root jobs from top to bottom.
- For each group, emit its jobs in their declared order before continuing to
  the next root item. An empty group emits no job.
- Copy the group's `on.change` patterns to each emitted job's `change`, then
  append that job's local `change` patterns. Do the same independently for
  `ignore`.
- Merge order is production order: inherited patterns first, local patterns
  second; remove only later duplicate occurrences. Do not sort or otherwise
  normalize patterns.
- Keep `ignore` as a separate surface. Matching remains ignore-wins, including
  when a path matches both effective change and ignore patterns.
- A job with no local patterns still receives the group's effective patterns.
  A root ordinary job keeps only its own effective patterns.
- Preserve each job's name (including tags), command scalar/list and bytes,
  `run_on_init`, cwd/env, service/output/recovery-compatible fields, trigger,
  timeout, and parallel metadata wherever the source parser accepts them.
- Group `on` contains common matching policy only. It does not become a global
  root `on` section, because doing so would change the scope of neighboring
  groups. Root-level policy and execution context are retained separately.

The transform is deterministic and idempotent. The second migration of a
converted file is byte-identical. Candidate validation uses the production
parser before an atomic replacement; parse, validation, or write failure
leaves the original bytes unchanged.

## Before/after behavior matrix

The migration changes configuration vocabulary and representation, not the
following observable behavior. TASK-0146/0147 must exercise the rows that are
relevant to each fixture.

| Surface | Legacy observation | V2 acceptance contract |
| --- | --- | --- |
| Watch | Matching root-list or group jobs runs the same commands. | `fzz -c FILE watch [TARGET]` selects the same jobs and effective patterns; no group is widened to a global root pattern. |
| Init | `run_on_init: true` runs in declaration order; ordinary jobs do not run at init. | Same flags, order, and one-time init behavior after wrapping/flattening. |
| List/explain/run | Names, tags, commands, and matching explanation expose the legacy jobs. | `list`, `explain`, and `run TARGET` expose identical job identities and command order; only `tasks` vocabulary becomes `jobs`. |
| Reload | Reload observes the same config path and later generations use the changed declaration. | Reload edits/adds V2-indented `jobs` entries; watcher identity, generation ordering, and replacement behavior stay unchanged. |
| Fail-fast | Jobs stop at the first failure in declaration order. | Flattened global order is the fail-fast barrier order; no group sorting or parallel widening is introduced. |
| Restart / non-block | A new matching batch cancels/restarts the active job according to the selected policy. | `--on-busy restart`/`--restart` sees the same ordered jobs and still terminates complete process groups. |
| Filepath templates | `{{filepath}}`, `{{relative_filepath}}`, and spacing variants are substituted in the same command sequence. | Command bytes and template tokens are preserved exactly; only the enclosing `jobs` structure changes. |
| Tags / target selection | Tags embedded in names filter the same quick/slow subset. | Names and `@tag` suffixes are unchanged; `watch`, `run`, and control target selection return the same subset. |
| Absolute paths | Absolute globs match/ignore the same files, including scratch-directory substitution seams. | Absolute pattern strings remain byte/order stable; ignore-wins behavior is unchanged. |
| Nested groups | Each group scopes only its jobs; group and local patterns are additive and ordered. | Each flattened job receives its group's effective patterns; group order, job order, barriers, and scope remain observable-equivalent. |

Runtime command failures are not config invalidity: intentionally failing
commands remain valid examples and must continue to demonstrate fail-fast and
failure reporting rather than being moved into `examples/invalid/`.

## Intentionally invalid fixtures

Invalid examples are V2-shaped in TASK-0147 and are never passed through an
in-place rewrite as if they were accepted public examples.

| Fixture | V2-invalid shape | Expected failure reason |
| --- | --- | --- |
| `invalid/invalid-value.yml` | `jobs:` wrapping the current items, retaining unquoted `change: **/hello/*` | YAML loader rejects the unknown anchor before config validation. This fixture must not be “fixed” by quoting the glob. |
| `invalid/missing-required-property.yml` | `jobs:` wrapping the current items, retaining the final `{run}` item without `name` | Production parser reports missing required `name`; it must not silently infer a name. |
| `invalid/non-list.yaml` | V2 `jobs:` plus `on:` as a sequence instead of an object | Production parser reports that `on` must be a Hash/Object; the `.yaml` extension remains covered. |

For each invalid fixture, `fzz check` must fail for the named reason and
`fzz migrate` must refuse without changing the original bytes. Invalid
fixtures are evidence of validation boundaries, not migration inputs.

## Public examples versus compatibility fixtures

- Public `examples/` files teach canonical V2 after TASK-0146. They contain no
  root YAML list and no `tasks:` key when valid.
- `examples/invalid/` contains only documented V2-invalid examples.
- Legacy root-list, grouped `tasks:`, mixed-shape, and nested-group inputs stay
  covered in `src/cli/migrate.rs`, `src/config.rs`, and dedicated migration
  tests. They are compatibility evidence and are not retained as duplicate
  public aliases.
- Inline YAML in unrelated tests is out of scope unless it references a
  renamed shipped filename or explicitly claims to validate the catalog.

## Required migration and validation gates

TASK-0145 must first add pure transform coverage for nested groups, mixed
regular jobs/groups, empty groups, comments, multiline/quoted commands, tags,
absolute patterns, and pattern merging. It must then prove one shipped nested
fixture through `fzz migrate` and `fzz check`.

TASK-0146 must apply the filename map, review every generated diff, recursively
check all valid `.yml` and `.yaml` files, and assert every already-V2 file is a
migration no-op. It must update active references atomically and leave no
legacy aliases.

TASK-0147 must add the recursive catalog gate, assert each invalid reason and
byte preservation, update `examples/README.md` with explicit V2 commands and
runnable/invalid separation, and run behavior rows for watch, init, list/
explain/run, reload, fail-fast, restart, templates, tags, absolute paths, and
nested groups.

## Non-goals

This contract does not rewrite examples, alter production config parsing or
runtime semantics, remove legacy compatibility, migrate unrelated inline YAML,
or add showcase fields merely to demonstrate V2.
