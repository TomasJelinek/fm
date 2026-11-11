# Native Jobs, decisions, Panel filter and Hot Paths

11 November 2026. Baseline: `5aee4d2`, authoritative `D:/experiments/dev/FM`.
This implements Task 3 and the Panel filter/Hot Paths subset of Task 4 in the
[accepted native-dialog plan](../superpowers/plans/2026-11-10-native-dialog-windows.md).
Source is implemented with **88 unique Windows checks** and **63 unique Linux
checks** passing. The Windows total includes one actual desktop Jobs/Decision
probe; the Linux checks use the software-window platform. The Windows debug application build, deployment, three
startup modes and focused lint pass. This note does not claim that every dialog
has migrated or that a broad suite passed.

## Implemented behavior

- **Jobs:** one modeless native presentation retains its selected job, scrolling
  and existing progress/chart model across refresh. Ordinary Close, Escape and
  native X close presentation only; Cancel job remains a separate explicit action.
  Manual Jobs stays open after completion. Existing automatic-success closure
  and retained job history remain in force. The permanent `closing` state is
  reserved for application shutdown, not ordinary hide/reopen.
- **Decisions:** Copy/Move errors and name conflicts, including metadata name
  conflicts, use modeless native prompts while Jobs remains available. Records
  carry exact operation identity/revision plus `DialogSession`, never row index
  authority. Escape and native X submit the exact Cancel decision; Decide later
  defers presentation without canceling the job. Rejected queue admission leaves
  the prompt, edited alternate name and warning available for retry. Failed host
  creation preserves the pending backend decision; Jobs Resolve explicitly retries.
- **Panel filter:** an owner-modal child retains its opening pane and captured
  navigation or tree root/session, with a separate form generation. Status updates
  and resize do not restart the form or overwrite its draft. Cancel invalidates
  pending validation. Apply and Show all retain existing backend checks and cannot
  act on a replacement navigation through stale callbacks.
- **Hot Paths:** one owner-modal manager captures a typed native/archive/Network
  bookmark location when opened. Add uses that capture rather than reparsing a
  displayed path or following a later active-pane change. Existing bookmark IDs,
  numbered-slot uniqueness and immediate persisted edits remain authoritative;
  closing the manager does not roll committed changes back.

The adapters reuse the owned native `DialogHost`, weak owner references, counted
modal leases and invoking-control focus restoration. Jobs has one record;
copy/metadata decisions have at most two; filter and Hot Paths have one each.
Backend jobs, validation and settings persistence remain outside the UI host.

## Review corrections included in source

1. Jobs Resolve releases its mutable Presenter borrow before native creation or
   activation. Decision opening can defer realization until the refresh hook when
   Presenter is still borrowed, avoiding synchronous `RefCell` reentry.
2. Metadata presentation preserves the explicitly selected live token. Selecting
   a different pending metadata job retires the prior UI occurrence even while
   its backend conflict remains valid; stale callbacks cannot address its replacement.
3. Ordinary Jobs close no longer invokes the old unconditional file-pane focus
   action, which would override the host's saved invoking editor. Shutdown and
   ordinary close use distinct paths.
4. Native decision bodies retain artwork when the duplicate embedded header is
   suppressed. Focused wrapper tests retain normal action-height, footer,
   recovery-text, icon-pixel and keyboard checks using four representative
   light/dark/high-contrast, Desktop/Touch and font-32 cases.
5. Failed Jobs/Decision creation now uses the existing bounded failure reporter
   to show an owned notice. The two visible-notice regressions first failed, then
   passed with the Rust-only correction. Backend jobs and decisions remain intact;
   only explicit Open/Resolve retries the failed presentation.
6. Hot Paths had the same transient-status failure path. Its injected native
   creation test failed at the missing visible Notice, while the Filter cleanup
   control passed. Hot Paths now shares the bounded owned-Notice reporter with a
   no-op backend callback and a shutdown gate. The Presenter borrow ends before
   reporting; only an explicit Open retries. Both failure cases pass on Windows
   and Linux, including compact visible text, no backend mutation and recovery.

The existing comparison popup also receives a narrow first-open focus correction.
At 320×200 with font 32, Linux focused an invisible, zero-width checkbox and Escape
could not reach the dialog. `comparison-dialog.slint::begin` now chooses the
reachable footer at the existing compact thresholds; the filter body follows the
same compact-focus policy. Directory comparison itself has **not** migrated to a
native window in this slice.

## Recorded evidence and pending checks

- Jobs/decision baseline: all **5** new integration cases failed at the missing
  independent-window assertion (zero child adapters). Log:
  `_build/qa/native-jobs-windows/red.log`.
- Filter/Hot Paths baseline: all **3** cases failed at the same missing-child
  boundary. Log: `_build/qa/native-filter-hotpath/red.log`.
- The compact comparison failure was reproduced in the preceding affected Linux
  run, then diagnosed as a focused item with geometry `0×75` and `is_visible=false`.
  The real Escape assertion remains. Log:
  `_build/qa/native-operation-windows/linux-dialogs-focused.log`.
- New/migrated targeted coverage is in `native_job_windows`,
  `native_filter_hotpath`, `native_tool_layout`, the affected Jobs/error/bookmark/
  filter fixtures, and the retained `dialog_drag` comparison regression. Final
  Linux results appear below; Windows desktop and deployment qualification are
  recorded by the integration owner. Prior checkpoint passes are not counted as
  verification of this source.

### First targeted Linux run

The four requested integration targets completed with **23 passed, 2 failed,
none ignored**. The failures are the strengthened native-creation diagnostic
checks, not ignored or counted as green:

| Target | Result |
| --- | --- |
| `dialog_drag` | 5 passed, including compact comparison first-open Escape |
| `native_filter_hotpath` | 8 passed |
| `native_tool_layout` | 3 passed |
| `native_job_windows` | 7 passed, 2 failed: visible Jobs/Decision creation notices |

Log: `_build/qa/native-tools-20261111/linux-targeted.log`. Tests used the pinned
SDK, one build job and private ext4 `TMPDIR`/`MIDNIGHT_NATIVE_TEST_ROOT`; no library
or full-suite target was included. Compilation took 10m01s and test bodies took
1.89s. The layout fixture's exact-label lookup now ignores the suppressed native
header, while retaining full-window and ancestor-scroll clipping checks on the
visible footer. The three wrapper cases retain actual colored-icon pixel checks,
normal Jobs button heights, minimum-window dismissal and full recovery text.

The initial compact comparison failure from the preceding checkpoint is now
resolved by its unchanged real Escape regression.

### Bounded reporter correction and Linux closure

Only `native_job_windows` and `copy_errors` were rebuilt and run after the
Rust-only failure reporter correction: **9 + 9 passed, none failed or ignored**.
The two previously failing cases now verify actual visible text in a 320×200
owned notice, preserved pending job/decision state and explicit-only retry.
The copy-error target also retains real Retry/Skip/Cancel input, exact-token queue
refusal, deferred-error restoration and automatic/manual Jobs closure policy.
Log: `_build/qa/native-tools-20261111/linux-jobs-green.log`; compilation took
3m21s with existing Slint bindings reused, and test bodies took 1.95s.

The unique Linux total at this checkpoint was **34 passed across five targets**:
`dialog_drag` 5, `native_filter_hotpath` 8, `native_tool_layout` 3,
`native_job_windows` 9, and `copy_errors` 9. The seven repeated Jobs passes are
counted once. The initial 23-pass/2-fail result remains above as evidence of the
diagnostic defect. No library/full-suite rerun was added. These are
software-window tests, not desktop window-manager qualification.

### Direct action-consumer boundary checks

Four directly affected Linux consumers were then run without changing their
fixtures or production code: `operations_native_ui` **3 passed**,
`tree_clipboard` **5 passed**, `tree_copy` **2 passed**, and `tree_move`
**6 passed**. The 16 cases retain actual child-window input and backend assertions
for F5/F6, permanent-delete confirmation/cancellation, folder drag, both-pane
clipboard operations, archive Move refusal, exact file bytes, selection changes
and retained tree sessions. Their native filesystem fixtures used the private
ext4 root; no permission or filesystem checks were weakened.

Log: `_build/qa/native-tools-20261111/linux-action-consumers.log`. Compilation
and linking took 3m51s, and test bodies took 26.87s. All four targets passed on
this first current-source run; no correction or rerun was needed. Including these
direct boundaries, this checkpoint reached **50 unique Linux passes across nine
targeted integration binaries**. The two later failure regressions brought the
next checkpoint to 52; final tree-consumer checks bring the total to 63 below.
This remains a scoped gate, not a full application suite or Linux desktop
qualification.

### Hot Paths failure regression and final targeted totals

The new two-case Linux RED run passed Filter's existing failed-create cleanup and
failed Hot Paths because it produced no visible diagnostic window after normal
refresh. Log: `_build/qa/native-filter-hotpath/linux-failure-red.log`. After the
Rust-only reporter correction, all **10** `native_filter_hotpath` cases passed on
both Windows and Linux. Their one-shot injected failure permits the fallback
Notice; it does not simulate failure of every native-window facility. The eight
previous cases remain unchanged and are counted once.

Final logs:

- `_build/qa/native-filter-hotpath/linux-failure-green.log`: 10 passed.
- `_build/qa/native-tools-20261111/windows-filter-final.log`: 10 passed.
- `_build/qa/native-tools-20261111/windows-hotpath-stale-final.log`: the updated
  retained-window callback fixture passed; included in the existing unit count.
- `_build/qa/native-tools-20261111/windows-jobs-failure-final.log`: both Jobs and
  Decision failure cases passed again; included in their original count.

Windows has **88 unique passing checks**, with repeated checks counted once:

| Scope | Unique passes |
| --- | ---: |
| Filter/Hot Paths: native integration 10, commander/bookmarks 3, stale-window callbacks 2, captured bookmarks 2, filter presenter 5 | 22 |
| Jobs/Decision integration and direct speed/scroll/reflow/popup/transfer checks | 34 |
| `dialog_drag` 5, `native_tool_layout` 3, `operations_native_ui` 3, `tree_clipboard` 5, `tree_copy` 2, `tree_move` 6 | 24 |
| Actual Windows Jobs/Decision desktop probe | 1 |
| Existing file-action conflict callbacks migrated to native decisions | 2 |
| Existing compact Jobs geometry and density/font cases migrated to native bodies | 2 |
| Existing native/archive tree-filter and calculated folder-size filter consumers | 3 |

The 34 Jobs/Decision checks and their logs are detailed in the
[Jobs/Decision evidence](2026-11-11-native-jobs-decisions.md). The earlier Filter
checks are recorded under `_build/qa/native-filter-hotpath/` in `affected.log`,
`panel_filter_windows-tests.log`, `hot_path_windows-tests.log`,
`presenter-bookmarks-capture_tests.log` and `presenter-filter-tests.log`.
The 24 direct Windows boundary cases passed in
`_build/qa/native-tools-20261111/windows-direct-consumers.log`.

Linux has **63 unique passes across eleven integration binaries**: `dialog_drag` 5,
`native_filter_hotpath` 10, `native_tool_layout` 3, `native_job_windows` 9,
`copy_errors` 9, `operations_native_ui` 3, `tree_clipboard` 5, `tree_copy` 2,
`tree_move` 6, `tree_filters` 4 and `tree_folder_sizes` 7. No failures or ignored
cases remain in this targeted set.

The actual Windows desktop probe passed 1/1 in
`_build/qa/native-tools-20261111/windows-native-jobs-desktop.log`. It checks owned
HWND styles without a separate taskbar entry, an enabled modeless owner, Jobs
native close without backend cancellation, and exact `Cancel 1:9` on the decision's
native WM_CLOSE. Background decision show and close preserve the observed
independent foreground HWND. Windows refused the first foreground setup attempt;
the corrected probe respects that restriction and observes an existing independent
foreground window when activation of its own test window is refused. It sends no
input to that observed window and makes no claim about a specific other app.

### Remaining fixture migrations

The final reference audit found obsolete embedded-popup assumptions in two
`file_actions` conflict cases and two `small_window` compact Jobs cases. The former
now address the actual native decision controls; **2/2 passed** in
`_build/qa/native-tools-20261111/file-actions-conflict-green.log`. The latter retain
their main panel and Options assertions while checking the native Jobs body on its
own adapter, including all prior density/font combinations, footer/action bounds,
visible captions, snapshots and actual Close. Both passed in
`_build/qa/native-tools-20261111/windows-small-window.log`. The snapshot shim then
reused the existing bitmap module to avoid duplicate module loading; its output
function is unchanged. These are fixture migrations, with no production or Slint
changes.

Linux's final tree-consumer RED run passed eight cases and failed three at the
obsolete Main-window Escape and embedded Filter-mode lookup paths. The fixtures
now dispatch through the visible native Filter adapter. All four `tree_filters`
and seven `tree_folder_sizes` cases then passed, preserving Arc identity, captured
pane/session, selection and no-extra-size-read assertions. Logs:
`_build/qa/native-tools-20261111/linux-tree-filter-{red,green}.log`. Compilation
took 2m03s and test bodies 10.59s. These 11 cases bring Linux to 63 unique passes.
Windows's three directly affected counterparts also passed, bringing Windows to
88 unique checks. Logs: `_build/qa/native-tools-20261111/windows-tree-native-filter.log`,
`windows-tree-archive-filter.log` and `windows-tree-size-filter.log`.

### Build, deployment and startup

The current Windows debug application build passed in **10.66 seconds**. The
matching runtime was registered and installed at
`C:/Users/tjelinek/MidnightExplorer-20261111-tools-0c91722c`. Executable SHA-256:
`0c91722cdcafdb9d52dac5e2c6d7e9fc64a60fe17905e94ab41e31f8e90cce03`.
Logs: `_build/qa/native-tools-20261111/build.log`, `deploy.log`, `runtime-path.txt`
and `app-sha256.txt`.

Backend, software-UI and native-UI startup checks each exited **0** with empty
stderr. Both `ui-software-startup.png` and `ui-native-startup.png` were visually
inspected by the integration owner. Their stdout/stderr logs are recorded under
`_build/qa/native-tools-20261111/` as `{backend,software,native}-startup.*.log`.
The owner also observed `_build/debug/me.exe` running as PID **53596**, titled
**Midnight Explorer 0.2**, with `Responding=true`. These are dated observations;
startup alone does not qualify every dialog's desktop placement or monitor changes.

Focused Clippy passed with `-D warnings` for the UI library and
`native_job_windows`, `native_filter_hotpath` and `native_tool_layout`. Log:
`_build/qa/native-tools-20261111/windows-changed-clippy-final.log`. The two existing
vendored FemtoVG warnings remain unchanged; no new project warning is reported.
Final fixture Clippy also passed with `-D warnings` for `file_actions_ui`,
`small_window`, `tree_filters` and `tree_folder_sizes`; log
`_build/qa/native-tools-20261111/windows-fixture-clippy.log`. The integration owner
confirmed formatting checks for the changed Rust files and `git diff --check`.
The final source-size gate passed **1,887 files**, all at or below **100,000 bytes**.
The later fixture edits do not change the deployed production executable.

Metadata editing, native directory comparison, Options, color picker and their
remaining forms still require migration. The earlier host foundation's native
Windows evidence does not replace qualification of these new adapters. Linux
software-renderer checks likewise do not establish X11/Wayland desktop placement
or focus behavior. No new broad test, workspace lint or performance run is claimed.
