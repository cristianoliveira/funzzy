# Funzzy User-Approved Job Recovery Contract

## 1. Scope and vocabulary

This contract defines bounded, interactive recovery for a failed configured job.
A recovery is an optional job-local command set. It is not a failure hook:
`hooks.failure` observes a final failed generation and cannot change its
result; a job recovery runs before the generation becomes terminal and may make
verification pass.

The terms below are normative:

- **original attempt**: the configured `run` command set executed for the
  generation before any recovery decision;
- **recovery**: the configured `recovery` command set for one job;
- **verification**: one rerun of that job's original `run` command set after a
  successful recovery;
- **recovery phase**: the approval, recovery, and verification work after the
  original attempt fails;
- **final result**: the generation result after the recovery decision and all
  permitted recovery work. Only this result is terminal.

MVP is deliberately bounded: one approval decision, at most one recovery pass, and
at most one verification per generation. There is no recursive recovery or
configurable retry loop.

## 2. Preferred configuration

The feature is available in the preferred V2 `jobs:` format:

```yaml
execution:
  recovery_policy: prompt # prompt | skip; default: prompt
  recovery_timeout: 60s # approval-only bound; default: 60s

jobs:
  - name: format-check
    run: cargo fmt --all -- --check
    recovery: cargo fmt --all
```

`jobs[].recovery` is optional and has the same command-list shape as
`jobs[].run`:

- a non-empty scalar is one shell command;
- a non-empty YAML sequence is an ordered list of shell commands;
- commands execute in declaration order;
- an empty scalar, empty sequence, non-string item, mapping, or null is
  invalid.

The recovery is job-local. It is not inherited from `on`, copied to other jobs, or
implicitly generated from a failed command. A `recovery` property in a legacy root
task list or under the legacy grouped `tasks:` vocabulary is not accepted by
this contract; `fzz migrate` never invents one. The V2 parser must report the
field path and job name for unsupported placement or invalid values.

`execution.recovery_policy` is an optional enum:

| Value | Meaning |
|---|---|
| `prompt` (default) | An eligible failure may ask the attached user once. |
| `skip` | Do not prompt and do not execute any recovery; preserve the failure. |

Unknown values and non-string values are configuration errors. `on.recovery_policy`, `hooks.recovery`, and a top-level `recovery_policy` are not aliases.

`execution.recovery_timeout` is a positive duration using the canonical `ms`, `s`, or `m` syntax. It defaults to `60s` and bounds only the local approval decision; recovery and verification commands keep their existing process and cancellation behavior. The budget starts when `approval_requested` is emitted and ends when a valid decision arrives, cancellation/supersession wins, or the budget expires. Expiry is default-deny: emit `approval_decided` with `approval timeout`, preserve the original failure, start no recovery or verification command, and finalize the generation as failed. Late input is ignored and cannot authorize this or a later generation.


A job with `service: true` cannot declare `recovery`. Configuration validation must
reject the combination at `jobs[].recovery` (or the equivalent job name/path),
because a service has no finite verification boundary.

## 3. Policy precedence and authorization

The exact CLI override is:

```text
--recovery-policy prompt|skip
```

The effective policy is, in order:

1. an explicit CLI `--recovery-policy` value;
2. `execution.recovery_policy` from the frozen configuration revision;
3. `prompt`.

The CLI option does not edit the configuration or persist across runs. It
applies to the configured workflow execution selected by that invocation or
watcher session. Commands with no configured jobs/recoveries have no recovery work;
ad-hoc `fzz exec` cannot acquire a configured job recovery.

Configuration declares that a command is available. It never authorizes a
mutation. Each execution requires the explicit interactive approval described
in §4. There is no automatic acceptance flag, environment switch, or implicit
trust based on the command text. In particular, `CI`, `NO_TTY`, and other
ambient environment variables do not change the effective policy.

`skip` is an immediate final-failure decision for a generation with an
eligible failed job: no prompt is shown, no recovery command is spawned, and the
original failure remains in the final diagnostic. The diagnostic must say
that recovery was skipped by `recovery_policy: skip` and identify the job so CI users
can choose an explicit policy or edit the configuration.

## 4. Approval prompt

After the original phase has reached a stable decision, a generation using
`prompt` asks at most once when one or more attempted jobs failed and declare a
recovery. The prompt must:

1. identify the exact generation number and every affected job's exact
   configured name;
2. render every recovery command in execution order, without shortening,
   reordering, or substituting a guessed command;
3. say that the commands may mutate the workspace; and
4. default to `No`.

A conforming human rendering is equivalent to:

```text
Generation 42 failed in jobs: format-check
Proposed recoveries (run once, in this order):
  [format-check] cargo fmt --all
Run these recoveries and verify the failed jobs? [y/N]
```

Only a trimmed, case-insensitive `y` or `yes` is affirmative. Empty input,
`n`, `no`, whitespace-only input, any other text, EOF, and an input error are
non-affirmative. The prompt is default-deny; an invalid answer is never
reinterpreted as approval.

The approval is a single-use authorization bound to the watcher instance,
configuration revision, exact generation, and exact ordered recovery set. A stale,
late, or repeated answer is ignored. Approval authorizes only the commands
rendered in that prompt; it cannot authorize a later generation, another job,
or a changed configuration.

A generation with failed jobs that have no `recovery` is not eligible for a prompt.
If both recoverable and unrecoverable jobs fail, the prompt lists only the recoverable
jobs and the unrecoverable original failures remain part of the final result.

## 5. Non-interactive and control-socket behavior

Approval is local to the watcher process. The watcher may wait while its
attached TTY is answering the prompt, but it must never wait forever for a
headless client:

- an explicit effective `skip` declines immediately;
- no attached TTY declines immediately;
- EOF, an invalid answer, an input error, or cancellation declines;
- a control-socket client cannot approve a recovery in this MVP.

A decline preserves the original job failure and emits an actionable reason
(`declined`, `no TTY`, `EOF`, `invalid answer`, `approval timeout`, or `cancelled`). A declined
recovery never spawns even its first recovery command. Remote clients may observe
that a generation is awaiting/processing a recovery decision, but they cannot
send an approval token or turn a stale response into authorization.

The generation remains non-terminal while an attached watcher is awaiting a
decision. `status`, `await`, and subscriptions therefore must not report the
initial failure as the final generation result. A headless watcher reaches the
final failure without blocking.

## 6. Bounded lifecycle and final result

The lifecycle is:

```text
original jobs
  -> original pass                         -> final pass
  -> original failure
       -> no eligible recovery / skip / decline -> final failure
       -> approve
            -> recovery commands once, fail-fast
                 -> recovery failure                    -> final failure
                 -> all recoveries pass
                      -> original job once
                           -> verification pass  -> final pass (if no other failure)
                           -> verification failure -> final failure
```

The recovery phase runs only after the original phase is quiescent. Recovery commands
for one job are sequential and fail-fast: the first spawn/nonzero/signal/
cancellation failure stops that job's remaining recovery commands and that job is
not verified. A failed recovery ends the recovery phase; later recoveries and verifications
are not started. This keeps mutation bounded and prevents a known-bad recovery
from authorizing more mutations.

When several jobs failed, all eligible recoveries are approved by the one prompt
and execute exclusively in declaration order. Each successful recovery is followed
by exactly one verification of that same job's original command set. A job
that had no recovery, was declined, or was not completed because fail-fast skipped
it remains non-passing. The generation passes only when the complete effective
outcome is passing; otherwise it fails.

The final generation outcome is the only outcome used for:

- `hooks.success` or `hooks.failure` (exactly once, and only after the final
  result; no hook runs for a superseded/cancelled generation);
- fail-fast generation exit behavior; and
- the terminal `finished`/`passed`/`failed` state exposed to clients.

The original failure, approval decision, recovery result, and verification result
remain observable diagnostics and phase events. They do not independently run
terminal hooks or create extra generations.

## 7. Execution equivalence and safety

A recovery uses the same resolved execution machinery as its job's `run` commands:

- the same workspace-root resolution and resolved `cwd`;
- the same job environment overlay and inherited process environment;
- the same template inputs (`{{filepath}}`, `{{paths}}`, and related values);
- the same shell command interpretation and command order;
- the same cancellation signal, grace period, process-group ownership, and
  descendant reaping; and
- the same configured output policy, bounded capture, log mirroring, and
  diagnostics.

Template expansion occurs for the frozen generation context. Unknown or
invalid templates are handled with the same explicit error policy as `run`;
Funzzy never silently replaces a recovery variable with an empty value. A recovery does
not run from the caller's current directory or inherit another job's context.

Recovery stdout/stderr is attributed to the same generation and job. Retained output
stays within the existing bounded output budget and uses the same exact task
identity; recovery output is not an unbounded second log store. Command text may be
shown in the local approval prompt, but secrets in environment values are not
printed or added to identity hashes.

## 8. Parallel and fail-fast scheduling

The original plan retains its existing serial stages, contiguous parallel
groups, concurrency cap, and fail-fast behavior. Recovery never runs concurrently
with original work or with another recovery:

1. every original attempt already admitted to a parallel stage is allowed to
   reach its existing terminal/cancelled state before recovery begins;
2. no later original stage is started once the existing fail-fast policy stops
   the original phase;
3. failed, actually attempted jobs are collected by declaration order;
4. the one prompt renders that ordered set; and
5. after approval, recoveries and verifications run one job at a time in declaration
   order.

A cancelled or skipped sibling is not an attempted failed job and has no recovery
run. A recovery cannot overlap a sibling's process, a service process, a later
stage, or another recovery. The configured concurrency cap applies to the original
phase; the recovery phase has an effective cap of one.

## 9. Cancellation, restart, and reload

Cancellation is authoritative at every phase. A cancelled generation never
runs a pending recovery, never accepts a pending answer, and runs neither terminal
hook. Under restart/busy replacement, the active generation is superseded,
its approval is invalidated, its owned processes are cancelled and reaped, and
the newer generation receives a new generation identity. Input typed after
that transition cannot authorize either generation.

Each generation captures one immutable configuration revision before its
original attempt. A reload that changes `recovery`, `execution.recovery_policy`,
`execution.recovery_timeout`, or the CLI-effective policy affects only later
generations. A pending approval keeps
using its captured job names, command set, revision, and policy; it is not
silently rewritten while the user is reading it. A malformed reload never
replaces the active revision.

## 10. Observability and protocol compatibility

The existing generation identity remains the sole identity for original,
recovery, and verification work. Implementations must make these phases
observable without pretending that the initial failure is terminal. The
additive phase vocabulary is:

```text
original_failed
approval_requested
approval_decided       (approved | declined | skipped | timed_out)
recovery_started
recovery_finished           (passed | failed | cancelled)
verification_started
verification_finished  (passed | failed | cancelled)
```

Phase records include the exact generation and job identity, declaration
order, phase result, and a safe reason. They must not expose environment
values or invent a second generation ID. Existing clients that ignore unknown
additive fields/events continue to observe one scheduled generation and one
final terminal result. The event schema/capability version is bumped or
extended according to the existing additive protocol policy before rollout;
old clients must not infer that `original_failed` is terminal.

Control-socket behavior for MVP is intentionally small:

- scheduling still returns the generation identity;
- status, await, and subscription surfaces remain correlated to that exact
  generation and report it as non-terminal during recovery;
- final failures retain the original failure plus recovery diagnostics and
  bounded output evidence; and
- no RPC method accepts approval, arbitrary recovery commands, or remote TTY input.

The pi-watcher contract must decode/ignore the additive recovery phase and
continue waiting for the final exact-generation result. It must explain that a
headless or remote watcher cannot approve a recovery instead of retrying approval
requests or guessing a CLI flag.

## 11. Compatibility surfaces and versioning impact

Implementation tasks must update and prove all of these together:

- **Parser and JSON Schema:** add `jobs[].recovery` with the same scalar/list
  command shape as `run`, `execution.recovery_policy` with enum/default `prompt`,
  strict unknown-property errors, and the `service: true` incompatibility;
- **canonical option catalog, `fzz config schema`, `fzz check`, init/example
  templates, and documentation:** show the preferred placement and the
  default-deny authorization rule; do not emit legacy aliases;
- **configuration reload/revision:** include recovery declarations and policy in
  the immutable runtime snapshot and semantic revision hash; formatting-only
  changes remain no-ops;
- **execution signatures and duration history:** include the effective recovery
  surface (recovery commands and policy, secret-safe) or version the signature
  explicitly so profiles with different recovery behavior never collide;
- **retained output:** preserve bounded generation/task identity and add phase
  metadata without a second unbounded store;
- **structured events and snapshots:** keep one generation and one final
  terminal event, add recovery phase observability, and negotiate additive
  fields/capabilities;
- **CLI:** expose exact `--recovery-policy prompt|skip` precedence and stable
  non-interactive diagnostics; never inspect ambient `CI`;
- **pi-watcher:** coordinate decoder/capability changes, with remote approval
  remaining unsupported; and
- **blocking and non-blocking watch paths plus local configured runs:** use one
  shared recovery policy so behavior does not depend on execution mode.

`fzz migrate` remains a V1 task-list to V2 `jobs` transform. It does not add a
recovery, infer a policy, or grant authorization. Existing configs without `recovery`
are behaviorally unchanged except that the default policy is available but has
nothing to recover.

## 12. Non-goals

This MVP does not provide:

- matching particular exit codes or output text to select a recovery;
- automatic acceptance flags or policies;
- magic `CI`/environment detection;
- remote/control-socket approval or an approval RPC;
- multiple recovery passes, recursive recovery, or configurable retries;
- generation-level recoveries that recover several jobs as one configured command;
- recoveries for unmanaged ad-hoc `exec` commands;
- recoveries on `service: true` jobs; or
- changes to the meaning of `hooks.failure`/`hooks.success` outside their
  final-generation boundary.

## 13. Contract proof matrix

| Case | Required result |
|---|---|
| no `recovery` | original failure; no prompt or recovery |
| `recovery_policy: skip` | immediate final failure; concise skip reason |
| prompt, no TTY | immediate final failure; no recovery process |
| prompt, EOF/invalid/declined | original failure preserved; no recovery process |
| approved one-command recovery passes, verification passes | one final pass |
| approved recovery command fails | final failure; no verification or later recovery |
| verification fails | final failure; no second prompt or recovery |
| two failed parallel jobs | all originals finish, one prompt, recoveries serial in declaration order |
| service with recovery | config error before execution |
| cancellation while prompting | approval invalidated; cancelled/superseded; no recovery |
| reload while prompting | captured revision/commands remain; later runs use new revision |
| final pass/failure | exactly one corresponding terminal hook |
| headless control client | observe only; cannot approve |
