# Session Jobs history and confirmation cancellation

The requested controls are **Clear all finished** and **Clear finished without
errors**. Both operate on the current application's display receipts across
Copy/Move, mutations and Custom Commands. They do not cancel work, delete files,
remove retained recovery data or persist changes to preferences. All unfinished
states remain, including queued, preparing, decision-waiting and cancelling jobs.
The second action additionally retains error states/counters, recorded errors
and unresolved Move source-recovery/durability problems.

Mutation preparation remains internally registered for shutdown cancellation and
bounded backend cleanup. Before confirmation its row is hidden. Cancelling that
confirmation retires the row only when the backend acknowledges cancellation;
preparation failures remain visible. Accepted confirmation releases the hidden
marker immediately, so cancelling queued/started work keeps its terminal receipt.
A bounded high-water mark rejects late progress for already-cleared mutation
receipts without retaining an unbounded set of historical IDs. Copy/Move drafts
already enqueue only on confirmation; their existing path is covered by explicit
Jobs-closed/no-history assertions. Custom-command retained preview allocations
continue to be reclaimed by the backend.

## Verification status

Focused tests cover clearing both finished policies across all unfinished states,
retaining cancellation/cleanup errors and Move residuals, late terminal receipts,
new completions after clearing, Custom Command errors/live work, Delete cancel
before/during confirmation, failed preparation and confirmed-job cancellation.
Modeless Copy/F6 tests assert Escape leaves Jobs closed and the history empty.

The coordinator's first targeted UI run passed all 11 `native_job_windows` tests
(`_build/qa/dialog-polish-20261111/ui-native_job_windows.log`), including actual
Clear actions, preservation of active work and detail-selection repair. All 6
`copy_dialog` tests passed (`_build/qa/dialog-polish-20261111/ui-copy_dialog.log`),
including F5/F6 Escape leaving Jobs closed with no history row. Scoped Rust
formatting and diff whitespace checks passed. These results cover the software-native window
boundary; they do not claim an interactive desktop check.
The v2 coordinated run reconfirmed `native_job_windows` 11/11 and `copy_dialog`
6/6 (`ui-v2-native_job_windows.log`, `ui-v2-copy_dialog.log` under the same QA
directory). `unit-v2-jobs_.log` passed 11 tests, with one existing interactive
Windows desktop case ignored; this includes all new Jobs-history, cancelled
Delete confirmation, visible preparation-error and Custom Command cleanup tests.
`unit-v2-job_history.log` separately records the original mixed-policy test pass.

Review then identified repeated per-ID retention scans when clearing a large
mutation history. The implementation now collects removed IDs once and visits
each progress/result/conflict collection once, with a single maximum-ID update.
Rename decision retirement uses the same batch predicate; live previews and the
metadata editor remain unless their own ID is removed. No Custom Command logic
changed. The new `jobs_clear_large_mixed_history_retires_related_receipts_once_and_preserves_live_authority`
regression covers 2,048 finished receipts alongside live and failed rows, all
related decision/result collections, pending preview identity, low live IDs below
the high-water mark, and late-receipt rejection. The post-optimization v3 run
passed both `job_history` tests (`_build/qa/dialog-polish-20261111/v3-ui-job_history.log`)
and all 3 Rename decision tests (`v3-ui-rename_decisions.log` in the same directory).
The 2,048-row regression completed within the 0.01-second test group, but no
separate latency benchmark or cross-platform performance guarantee is claimed.
