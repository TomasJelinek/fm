# Tree diagnostic details, 2026-11-10

This record covers backend retention of full file-panel tree failures. The
coordinator owns the combined Windows/UI qualification and deployment record.
It does not claim that a newly built application has been deployed.

`TreeDisplayRow.error` and `TreeDiagnostic::as_str()` retain short bounded
presentation text. Their separate generated-text source preserves the complete
provider description before the 4096-byte tree limits. Native provider failures
also preserve original operation/reason, error kind and escaped raw native path
units before conversion into provider text; no native path becomes an operation
target through diagnostic output.

Each tree generation uses a lazy shared private append spool. Failed nodes retain
immutable record ranges; retries clear current diagnostic state without changing
an already opened report. Tree checkpoints/publications retain only source
owners. Old appended bytes remain until generation retirement and the final
retained reader releases them. Retained aggregation uses bounded summaries and
source owners instead of accumulating complete error strings in memory. An
incoming provider error, and its formatting before spooling, can still occupy
memory proportional to that individual error's length. Disk storage failure
reports observed/stored counts and missing details explicitly.

A native maintenance refresh can interleave explicit branch work. Its summary
therefore composes the exact immutable sources of every failed branch, bounded
by the retained tree's node count, rather than taking an imprecise shared-log
range. Its vector contains only source owners bounded by the tree-node cap,
without copied error/path strings or a file per failure.
An aborted refresh retains earlier failures and its terminal failure.

Dynamic terminal construction/rejection/navigation errors also retain full
sources. `TreeDiagnostic` equality compares bounded text and retained snapshot
identity without reading files. The presenter must preserve exact pane/error
correlation and send rejected/replaced sources to its existing backend reclaimer;
these source owners cannot be finally destroyed on the UI thread.

## Evidence

All commands use the existing offline SDK config and `_build/linux` target,
with two Cargo jobs and the existing Linux environment script. No dependencies
or provider/plugin ABI changes were introduced.

- `_build/qa/size-warnings/tree-failure-red-linux.log`: actual failed provider
  listing loses the full long Unicode description. Expected regression failure.
- `tree-refresh-red-linux.log`: two failed sibling folders lose their refresh
  detail source. Expected regression failure.
- `full-libs-green-linux.log`: core **51 passed**, engine **678 passed**, seven
  existing installed-catalog opt-ins ignored. This includes full Unicode tails,
  later sibling isolation, immutable reports after retry/tree destruction,
  terminal errors, explicit storage failure and every refresh branch failure.
  It also covers native/archive/tree size-warning and cancellation integration.
- `full-spool-green-linux.log`: worker-platform **21 passed**, two unrelated
  native cgroup environment tests ignored. Includes immutable-prefix ownership,
  late partial writes and serialized concurrent reader/append behavior.
- `engine-routing-green-linux.log`: **2 passed** through the real Engine
  command/event route for full/individual scans, blocked-call cancellation,
  acknowledgment, late frozen reports and generated source reads.
- `native-provider-red-linux.log`: **3 expected failures** independently expose
  lost native kind, merged invalid-UTF8 paths and lost final archive revision error.
- `diagnostic-race-red-linux.log`: **3 expected failures** expose real errors
  dropped during cancellation, fabricated Interrupted warnings and a fresh source
  causing unchanged cached watch failures to dirty the watch again.
- `final-engine-green-linux.log`: **684 passed**, seven existing opt-ins ignored;
  this is the final backend snapshot after all those corrections. The watch cache
  reuses its frozen diagnostic under the existing 15-second policy; scheduling is
  unchanged. Session 32023 exited 0.

Initial RED attempts stopped on shared migration compile errors; those are not
counted as behavior evidence. Completed sessions: 42927 and 74133, both exit 0.
Source before-images and detailed inspection live under
`_build/qa/size-warnings/tree-failure-baseline/` and `tree-failure-review.md`.

## Integrated Windows/UI qualification

The Windows unpaired-UTF16 counterpart and full engine tests pass. The Windows
gate additionally exposed and fixed concurrent first-use creation of private
report storage without relaxing its owner/DACL/reparse checks. Tooltip/detail
gestures, keyboard/accessibility, exact occurrence correlation and full Copy
are covered by the integrated UI tests. Counts, remaining desktop limits,
strict lint and deployment are recorded in the
[combined diagnostic qualification](2026-11-10-panel-warning-details.md).

Read-only Salamander comparison: `src/mainwnd5.cpp` directory-read failures
around lines 581/703 format the actual path and `GetErrorText` in an exclamation
message box; `src/fileswn3.cpp` uses the OS reason for directory enumeration.
This supports specific path/reason diagnostics. Its legacy buffer truncation,
silent size-count error skipping and absence of retained clickable reports are
not adopted.
