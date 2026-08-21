# TASK-0123 handover: user-approved job recovery

Date: 2026-08-20

## Current state

TASK-0120 and TASK-0121 are complete. TASK-0122 is committed and moved to
`plans/done/`. TASK-0123 remains `doing`; it is the only file in
`plans/todo/`.

The worktree was clean when this handover was created.

## Contract and terminology

- Contract: [`JOB-RECOVERY-CONTRACT.md`](JOB-RECOVERY-CONTRACT.md)
- Use `recovery`, `recovery_policy`, and `--recovery-policy`.
- Recovery is bounded, default-deny, and explicitly approved.
- One approval, one recovery pass, and one verification per generation.

## Relevant commits

- `be34e7c` — TASK-0120 recovery contract
- `7d99711` — TASK-0121 parser/model/schema support
- `94a96ff` — TASK-0122 executor lifecycle
- `793966b` — recovery documentation and example
- `0e91f70`, `eee4ce0` — pseudo-TTY approval coverage for both binaries
- `81e3af2` — actionable approval safety decisions
- `0b9f22a`, `224ee0f` — recovery failure, multi-command, and skip coverage
- `30627a2` — parallel declaration-order coverage
- `bf2af99` — bounded lifecycle-event coverage
- `b47aff9` — frozen revision/job-position approval assertions

## Implemented behavior

- Injected `RecoveryApproval` boundary with a TTY adapter and safe headless
  default denial.
- Added `ApprovalDecision::{Approved, Declined, NoTty, Eof, Invalid}`.
- CLI/config policy precedence is wired through finite and watch execution.
- Approved recovery commands run sequentially and fail-fast, followed by one
  verification of the original command list.
- Recovery requests carry generation, revision, job position/name, and exact
  rendered commands.
- Recovery phase events are emitted without duplicate terminal task events.
- README, usage, advanced guide, init template, and a formatting example are
  updated.

## Verification already run

- `cargo test --lib` — passed in watcher generation 27
- `cargo test --test recovery_cli` — passed
- `cargo test --test recovery_pty -- --nocapture` — passed
- Full watcher generation 27 — passed and fresh

## Remaining acceptance work

1. **Cancellation/restart invalidation**
   - Current `RecoveryApproval::approve` is synchronous. A worker cannot
     process a cancellation command while it is blocked in an approval read.
   - Design a cancellation-aware approval boundary/token before claiming this
     criterion. A late answer must not authorize a superseded generation.
   - Prove cancellation during recovery/verification also reaps the child and
     suppresses terminal hooks.

2. **Hot reload freezing**
   - Add an integration test that changes/removes/replaces `recovery` and
     `execution.recovery_policy` while an older generation is active or awaiting
     approval. The older generation must retain its original commands/revision;
     only later generations use the candidate revision.

3. **Observability and hooks**
   - Add end-to-end assertions for retained output, snapshots/failure lists,
     structured recovery phase events, and exactly-once success/failure hooks.
   - Prove `show-on-failure` does not reveal evidence until the final result is
     failure, while structured/verbose output remains inspectable after success.

4. **Parallel safety**
   - Unit coverage proves approval request declaration order. Add a process-level
     test proving no recovery overlaps an original parallel sibling or another
     recovery.

Do not move TASK-0123 to `plans/done/` until these remaining criteria are either
implemented and tested or explicitly renegotiated.
