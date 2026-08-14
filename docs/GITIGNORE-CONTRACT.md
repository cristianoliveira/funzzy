# Funzzy Gitignore Respect Contract

> Status: **normative** — defined by TASK-0036. Drives the `respect_gitignore`
> config knob, the `GitignoreMatcher`, and explain output.
> Implementation: `src/gitignore.rs` on the established `ignore` crate
> (same semantics as git/ripgrep).

## 1. Default and override

- **Default: `respect_gitignore: false`.** Existing configurations keep today's
  exact behavior; nothing is silently ignored when the knob is absent. This
  makes the compatibility surface explicit, not surprising.
- `on.respect_gitignore: true` enables workspace `.gitignore` matching.
- Precedence (strongest first):
  1. **Explicit config `ignore` rules** (job-level and `on.ignore`) — always
     win; migration is not required.
  2. Gitignore rules when `respect_gitignore: true`.
- An explicit watch rule never loses to gitignore: if a job's `change` pattern
  matches a path but a config `ignore` rule does not, gitignore may still
  exclude it; the config `ignore` is the user's explicit contract and remains
  strongest.

## 2. Semantics

- Uses the `ignore` crate: nested `.gitignore` files, negation (`!pattern`),
  anchored rules (`/pattern`), directory rules, and global excludes resolve
  like git/ripgrep.
- A path that matches a negated rule is **not** ignored.
- Paths outside the workspace root are never gitignored (the matcher is
  root-anchored); config `ignore` still applies to them.
- Nested repositories: a nested `.gitignore` applies under its directory,
  matching git's parent-chain resolution.

## 3. Explain and diagnostics

- `fzz explain PATH` names the exact ignore source: the config rule
  (`ignored by: <pattern>`) when the config ignored it, or the gitignore
  source (`.gitignore`) when gitignore excluded it.
- Deterministic across equivalent relative/absolute paths: matching is always
  performed on the root-relative path.

## 4. Cache and reload

- The matcher is built once per workspace root and cached; matching never
  rescans ignore files per event (the `ignore` crate holds parsed rules).
- When the workspace `.gitignore` mtime changes, `needs_rebuild()` is true;
  the watcher rebuilds the matcher **before** routing the next batch, so no
  event-loss gap occurs. The old matcher stays valid until the rebuild.

## 5. Out of scope

- Editing or generating `.gitignore` files.
- `.git/info/exclude` and global git excludes (the `ignore` crate can add
  them; this contract keeps scope to the workspace `.gitignore` set).
