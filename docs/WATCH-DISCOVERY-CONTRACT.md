# Funzzy Watch Discovery Contract

> Status: **normative** — defined by TASK-0085. Drives TASK-0086 (stable
> ancestor root planning), TASK-0087 (new-path routing proof), and the
> config-reload semantics of TASK-0088..0092. Interacts with
> `docs/GITIGNORE-CONTRACT.md` (TASK-0036), the debounce/batch identity
> contract (TASK-0031, `src/watcher.rs` `normalize_batch`), and the backend
> policy (TASK-0037, `src/watcher.rs` `WatchBackend`).
> Implementation anchor: `src/watches.rs` (root planning + matching),
> `src/watcher.rs` (backend adapter), `src/watch_loop.rs` (one routing flow).

Users configure path patterns before all files and directories exist. A
watcher that subscribes only to startup-resolved paths can silently miss
later creations. This contract makes future-path coverage an explicit,
deterministic property: what is subscribed, what is observed, what matches,
and what is merely configured are five distinct facts that never blur.

## §1 Vocabulary — five distinct facts

| Fact | Definition | Example | Owned by |
|---|---|---|---|
| **Configured pattern** | User intent: the `change` (and `ignore`) glob strings a job declares, plus `on.ignore` global ignore rules. | `change: 'src/**'` | config |
| **Subscription root** | A concrete directory the backend is told to watch at startup (or after reload). Derived deterministically from patterns; the *only* thing the backend registers. | `/repo/src` | `Watches::paths_to_watch` |
| **Observed path** | A normalized filesystem path that reached the routing flow inside one debounce batch. | `/repo/src/lib.rs` | `watcher.rs` batch normalization |
| **Matched job** | A task selected because the observed path matched its change pattern and no ignore won. | `build` | `Watches::watch_plan(_batch)` |
| **Git tracking** | Whether the path is excluded by workspace `.gitignore` when `respect_gitignore: true`. Never about `git add`/staging. | `.gitignore` entry | `GitignoreMatcher` |

Rules:

- A configured pattern is **not** a subscription root. The root is the
  deterministic literal-prefix of the pattern (see §3). Patterns are matched
  against observed paths, never against root lists.
- An observed path is **not** a matched job. Observation happens in the
  backend; matching happens once, in the routing flow (§2). Backends never
  decide jobs.
- Git tracking applies only when `respect_gitignore: true` and only to
  workspace-root-relative paths (GITIGNORE-CONTRACT §2). It is one
  predicate in the ignore step, weaker than explicit config `ignore`.

## §2 Uniform routing flow — one flow for every event kind

Creation, modification, and removal all route through the **same** flow.
There is no separate "create" execution path:

```text
normalize (dedup, sort, kind) → ignore (config ignore, then gitignore)
  → match (change patterns) → batch (one debounce window → zero/one generation)
  → busy policy (restart | wait)
```

- A debounce window is one normalized batch (`normalize_batch`): paths are
  deduplicated and deterministically sorted; the batch maps to zero or one
  generation (`watch_plan_batch`, contract §1 of the one-hop output
  contract). The trigger is the deterministic first match in sorted order.
- Event kind (`any` / `continuous`) never changes matching. A `create` and a
  `modify` of the same path are indistinguishable to the routing flow.
- A batch whose paths are all unmatched or all ignored yields **no**
  generation — an explicit no-op, never a synthetic success.

## §3 Subscription-root planning — nearest existing ancestor

A pattern's literal directory prefix is the candidate root: the longest
path-prefix segments that contain no glob metacharacter
(`* ? [ {`), e.g. `src/**` → `src`, `examples/workdir/**/*` →
`examples/workdir`, `/tmp/funzzy-*/*.txt` → `/tmp`.

- **Existing prefix:** watch it (recursive on native, recursive-scan on
  poll). Nothing above it is watched for this pattern.
- **Partly/nonexistent prefix** (`future/deep/src/**` when
  `future/deep/src` does not exist): watch the **nearest existing ancestor**
  (`future` or `future/deep`). Matching for that pattern begins automatically
  when the missing descendants appear — creation under the ancestor is an
  observed path and routes normally (§2). No watcher restart, no "touch a
  file to arm it".
- **Root fallback is bounded:** the workspace root is watched only when a
  narrower safe ancestor does not exist. Recursively watching the root
  because *one* pattern is future-heavy is never the default
  (TASK-0086 enforces the minimal set).
- The root set is canonicalized, deduplicated, containment-minimized (a root
  inside an already-watched root is dropped), workspace-bounded for relative
  patterns, and **stable** — independent of hash/map iteration order
  (TASK-0086 pure tests).
- Subscription does not traverse symlink cycles, `.git` directories,
  socket/state/log outputs, or ignored trees. Resource and cycle behavior is
  bounded; never watch a path the pattern cannot match.
- Absolute patterns may point outside the workspace root; their prefix
  resolution follows the same nearest-existing-ancestor rule, and paths
  outside the root are never gitignored (config `ignore` still applies).

## §4 Creation of trees and atomic saves

- **Directory tree + file in one operation:** intermediate directory events
  are observed like any path. A directory that matches no change pattern is
  inert. The canonical final file path is the path that routes; intermediate
  events do not run unrelated jobs, because one batch routes through the
  deterministic first-match rule and directories rarely match file globs.
- **Atomic editor save** (temp create/write/rename over destination): the
  debounce batch may contain both the temp path and the final path. The
  destination semantics apply **once per batch** — when the backend supplies
  a rename, the final destination is the matching path; duplicate temp/final
  events are deterministically deduped within the configured debounce
  (TASK-0086). A temp/ignored path never leaks as the selected job: if the
  temp name matches an ignore rule it is dropped in the ignore step; if it
  would sort before the destination, the destination still wins because the
  atomic-save rule prefers the final path over a same-batch temp sibling.
- Polling backends see renames as remove + create; the matched-path outcome
  is still the final destination (same normalized path set after the
  baseline), so the job fires on the destination once.

## §5 Deletion and recreation

- Deleting a file or directory, then recreating it, remains observable
  **without watcher restart**. The subscription root is the ancestor
  (stable), not the deleted object; recreation under a watched ancestor is a
  normal observed path.
- Native backends keep the recursive ancestor subscription across the
  delete (the parent directory is the root). Poll backends detect the
  existence change on the next scan. Neither keeps a stale assumption about
  the deleted path.
- A pattern whose **literal prefix** is deleted entirely falls back to the
  nearest existing ancestor on the next root-plan refresh (config reload or
  restart); within a running instance the ancestor watch already covers it.

## §6 Precedence and policy interactions

Precedence, strongest first:

1. **Explicit config `ignore`** (job-level and `on.ignore`) — always wins.
2. **Gitignore** when `respect_gitignore: true` (GITIGNORE-CONTRACT §1).
3. Change-pattern matching.

Additional explicit policies:

- **Symlinks:** matched by their reported path; subscription never follows a
  symlink cycle; a symlink whose target lies inside a watched root is
  observed under its link path. Backends may report either form, but the
  matched-path outcome is equivalent (§7).
- **Workspace escape:** relative patterns anchor to the workspace root and
  never match outside it; absolute patterns may match outside; gitignore
  never applies outside the root.
- **Hidden paths:** dotfiles and dot-directories are ordinary paths for
  matching — no implicit ignore of hidden names; `**` includes them unless a
  rule says otherwise (tooling caches are excluded by explicit config
  `ignore`, not by magic).
- **Config-file reload:** a valid reload atomically replaces the
  subscription-root plan and execution policy; old roots stop routing after
  the instance boundary and control identity is preserved (TASK-0088..0092).
  An invalid reload kills the watcher with a nonzero exit — never a silent
  half-subscribed state. Until hot reload ships, `--restart` remains the
  reload path and the same root-plan rules apply after restart.

## §7 Backend equivalence

Native and polling backends promise **equivalent matched-path outcome**: for
any deterministic sequence of filesystem operations, the set of generations
scheduled and the jobs matched are the same. Raw event counts, event order,
and event kinds are **not** contractual — the normalized batch and the
deterministic first-match rule absorb backend differences.

- Poll discovers additions/removals recursively under the same bounded roots
  and never emits baseline contents as changes (first scan seeds the
  baseline, reports nothing — `PollScanner`).
- Both backends feed the identical `normalize → ignore → match → batch →
  busy-policy` flow (§2). The backend is an adapter, never a policy.

## §8 Diagnostics

- Verbose startup emits one `watch_root` record per subscription root
  (source `config`, decision `watch_root`), so operators see exactly what the
  backend registered — not what was merely configured.
- `explain PATH` reports matched/ignored/unmatched per rule
  (`src/watches.rs::explain`); it additionally names, for a future path, the
  subscription root that will observe it and why the missing prefix is
  covered (TASK-0086).
- A **truly unwatchable root** (e.g. permission-denied, non-existent
  absolute prefix with no existing ancestor) fails or warns actionably —
  never a silent miss. The current native path warns
  (`unknown file/directory: '…'`) and continues for missing paths; hard IO
  failures return an error (`failed to watch path: …`).

## §9 Synthetic `emit`

`emit` (`fzz control emit`, `WatchRunner::emit_path`) routes a synthetic path
through the **exact same** matching policy as a filesystem event:
normalization, change match, ignore precedence, task ordering, and the
cancel-and-schedule busy contract. For a nonexistent or future path it is
routing-equivalent — matching is path-based and never requires the path to
exist.

`emit` **never claims filesystem subscription proof**: it does not assert
that the path is covered by a subscription root, does not create or touch
files, and its diagnostics state the outcome (`scheduled` / `unmatched` /
`ignored`) without implying the backend observed anything. Subscription
coverage is a startup/explain diagnostic (§8), not an `emit` assertion.

## Out of scope

- Editing or generating `.gitignore` files (see GITIGNORE-CONTRACT).
- Git staging/tracking semantics beyond the ignore predicate.
- Network/filesystem-notification transport beyond native/poll.
- Retrying failed events or crash recovery of partially-applied configs
  (reload is atomic or fatal, §6).
