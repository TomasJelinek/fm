# Operation progress cancellation confirmation

The shared Copy/Move/Delete/Recycle progress window now asks before cancellation
from native X, Escape or its Cancel operation control. The question replaces only
that existing window's interactive body. Continue receives keyboard focus; Enter,
Continue, question Escape and question X preserve the job's current running or
paused state. No extra native window, automatic Pause/Resume, or global modal
state is introduced. Jobs history dismissal is unchanged.

Only explicit confirmation dispatches cancellation. The callback checks its exact
window session and job identity, question state, and the latest active/cancellable
row. Terminal updates clear the question; automatic successful windows retain
their existing close policy. A rejected backend enqueue keeps the question and
retry feedback visible. Other operations and the main window remain usable.

## Regression and implementation

The actual native-window harness reproduced the old behavior: Escape immediately
sent cancellation. `confirmation::progress_escape_requests_confirmation_without_cancelling`
failed its no-command assertion in 0.04 s before production changes (build 5.53 s). Evidence:
`_build/qa/progress-confirmation-20261112/windows-red.log` and `red.ps1`.

Focused regressions cover Copy, Move, permanent Delete and Trash; both native and
keyboard close routes; Continue as the default; explicit confirmation; rejected
admission; terminal updates; callbacks retained across window replacement; paused
work and independent operations. The direct rendered component checks both actions
at 640 × 420 and 320 × 200 in desktop/touch themes, with real keyboard and pointer
input. Render artifacts are kept under the QA directory, not committed.

Implementation is confined to `job_progress_windows.rs`,
`ui/job-progress-window.slint`, and their focused tests. It reuses the existing
header, footer and action buttons. Confirmation fences other callbacks from that
window, including retained callbacks from the removed interactive body.

## Qualification

| Platform / target | Result |
| --- | --- |
| Windows `native_job_windows` | 19 passed in 0.79 s. |
| Windows `job_progress` | 3 passed in 0.35 s. |
| Linux `native_job_windows` | 19 passed in 6.12 s; includes the corrected stale-window fixture. |
| Linux `job_progress` | 3 passed in 1.73 s. |
| Linux `native_operation_windows` | 10 passed in 2.74 s in the final targeted rerun. |

Windows evidence is `_build/qa/progress-confirmation-20261112/windows-green.log`
(build 7 min 55 s), with `green.ps1` and `windows-executables.sha256` alongside it.
The same session reran the adjacent corrected F2/F7 integration: `tree_rename`
3 passed in 2.12 s; its evidence belongs to
`_build/qa/tree-rename-20261112/ui-keys-final.log`.

Linux evidence is under `_build/qa/config-rename-cmd-20261112/`:
`linux-ui-native_job_windows.log`, `linux-ui-job_progress.log` and
`linux-ui-native-final.log`. The first broader run exposed two older
`native_operation_windows` assertions expecting immediate cancellation. Their
preserved initial log is `linux-ui-native_operation_windows.log`. Updated
fixtures first assert that the question sends nothing, then explicitly confirm,
preserving the original exact-target/destination checks and rejected-queue retry.
No production correction was needed for these legacy expectations.

Actual software renders at 320 × 200 in desktop and touch/high-contrast themes
were inspected on both platforms. The question is readable, both actions are
visible and Continue has the focus outline and neutral checkmark. Representative
artifacts are `renders/cancel-question-21-1-320x200-320x200.bmp` in the Windows QA
directory and `linux-render/cancel-question-0-0-320x200-320x200.bmp` in the combined
Linux QA directory. No screenshots are committed.

Independent read-only review found no remaining cancellation authority or
per-window modality issue. Parent review corrected the inherited X artwork on
Continue to the existing neutral checkmark before final qualification. The final
Windows affected UI gate also passed all 10 `native_operation_windows` cases in
0.13 s. Combined Windows/Linux engine and affected UI gates, strict all-target
Clippy, formatting, fresh debug build and registered-runtime startup checks passed.
See the [combined checkpoint](2026-11-12-config-rename-cmd-checkpoint.md) for exact
counts, scope, executable identity and remaining qualification boundaries.
