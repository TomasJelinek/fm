# Linux desktop fixes and Colors hang investigation — 13 November 2026

Scope: `codex/filter-linux-drive-list`, based on merge commit
`2b62996e60a175ec0ecf9bbefd79718fd3666115`, with uncommitted desktop fixes.
This evidence does not qualify unrelated work in the main development ledger.
No dependencies or enabled dependency features changed.

## Implemented behavior

- A per-user desktop installer registers the existing Debug executable and
  matching application ID/icon for GNOME. The desktop entry and icon were
  installed for this checkout; Gio and GTK resolved both successfully.
- Attributes offers Owner/Group/Other read/write/execute, special bits,
  synchronized octal/text and mixed-value handling. Untouched bits remain intact.
  The production initializer installs the same codec exercised by widget tests.
- Linux terminals select the configured executable shell and preserve normal
  interactive startup files. Basic prompts gain user/host/current-directory
  defaults using shell-native or POSIX mechanisms.
- X11 restoration supplies `USPosition` before the first map and retains the
  existing size hints. If native position is unavailable, saved coordinates and
  their monitor identity stay together. Wayland absolute placement remains a
  compositor restriction; it is not claimed as qualified restoration.
- Explicit activation runs executable ELF files directly and executable shebang
  or `.sh` scripts in a terminal. Native paths remain literal arguments; no
  execute permission is added. Ordinary documents retain host associations.
- Ctrl+click on the filename in the bottom details bar copies its exact full
  native path. Plain click still copies the filename. The tooltip explains both.
  Stale, virtual and unrepresentable native identities are rejected.

## Verified evidence

All new compilation in this scope used **Debug**, with `_build/` as the Cargo
target directory and the existing offline Linux build image. Root held the build
lease; desktop tests ran serially. Artifacts are under
`_build/qa/ubuntu-permissions/`.

| Boundary | Result | Evidence |
| --- | --- | --- |
| Linux desktop installer, including Gio parsing and icon-cache behavior | 16 passed | `scripts/tests/test_install_linux_desktop.py`; installer registration/resolution checked separately |
| Linux shell selection/prompt and executable activation | 30 passed | `activation-terminal-tests.log` (`desktop::` and `open::`) |
| Attributes integration, production codec installation and status-bar pointer/tooltip behavior | 24 passed | `final-tests/summary.json`, five integration targets |
| Metadata input, permission projection, native placement hints and status identity guards | 21 passed | `final-tests/summary.json`, filtered `midnight_ui` library target |
| Window-position unavailable-origin/monitor policy | 8 passed | `final-tests/summary.json`, filtered `me` target |
| Native X11 first map, move, repeated capture, maximize/restore and final captured save | 1 passed | `native-position-test.log`; separately owned software-rendered window on the current GNOME/X11 session |
| Colors priority navigation, including the newly reported default groups/tooltip/Down workflow | 6 passed | `color-navigation-tests/summary.json` and its target log |
| Actual Debug executable and elevation helper build | exit 0 | `final-debug-build.jsonl` |
| Native graphics startup and automatic shutdown of the Debug executable | exit 0 | `smoke/native-ui.log`, `smoke/debug-startup.png`; in-memory settings, owned empty directories |
| Formatting, whitespace and source-size checks | passed | `cargo fmt --all -- --check`, `git diff --check`, `bash scripts/check-source-size.sh` |

Total: **106 targeted tests passed**, plus the native startup smoke run. This is
not a full-workspace test result. Safe script fixtures exercised actual execution;
the user's `ovclash-process-mb-h56.sh` was neither read nor executed. Shell probes
also exercised a controlling PTY, Bash, sh and the installed BusyBox ash; this
does not claim physical testing on every Linux distribution or shell. The actual
GUI terminal-emulator handoff and interactive sudo authentication were not tested.

Native placement observed first-map bounds `(180, 150, 720, 460)` and final normal
bounds `(253, 201, 720, 460)`, with no capture drift. The current native graphics
smoke emitted WGPU's `MAPPABLE_PRIMARY_BUFFERS` discrete-GPU warning and exited
successfully; this warning alone does not establish a cause for the reported hang.

## Earlier Colors report: process restarted before capture

The user reported an unresponsive Options → Colors window while selecting an
extension group with Down. The process available during inspection was PID
`278880`, running the older `_build/release/me` built at the base merge commit.
The user then confirmed that the original hung process had already been restarted.
The original thread stacks therefore could not be recovered.

The replacement process's main thread was sampled in `ep_poll`. That describes
only the replacement process, not the original hang. Debugger attachment and
protected `/proc` stack/syscall reads were denied by the host's ptrace permissions;
passwordless sudo was unavailable. Logs are in `live-hang/`. No process was killed
or restarted by the investigation, and no ptrace policy was changed.

Source inspection found no blocking I/O or lock wait in the bounded rule-selection
callback. Popup/focus handling is an unconfirmed hypothesis. The new software UI
test reproduces default colors, a real tooltip, repeated Down/Up/Home/End,
selection/draft synchronization, scrolling and closing Options; it passes.
Native startup also passes, but native Colors interaction and the original hang
are **not** thereby qualified. No speculative production fix was made for this
report. That earlier occurrence remains unqualified. The later live capture below
provides evidence for a subsequent hang.

## Later live hang: panic followed by runtime shutdown wait

The user subsequently requested inspection while the application was still hung.
The captured Debug process, PID `342464`, retained a visible native window. Its
main thread was in Tokio's `BlockingPool::shutdown`, reached through `Runtime`
destruction during unwinding from `me::run`. A blocking image-owner thread was
idle in `image_startup::Shared::next_work`, waiting on its condition variable.
A retained viewer-service clone prevented ordinary destruction from stopping
that owner. The second process, PID `368132`, was sampled in the normal Winit/Slint
event loop. Both debugger sessions detached; neither process was terminated or
restarted, and no ptrace policy was changed.

The journal records a main-thread panic at **13:06:36 +01:00** in Slint 1.18.1,
`window.rs:2252:52`: **`RefCell already borrowed`**. The original panic backtrace
was not enabled, so the exact original UI callback is unavailable. Captured stacks
and the recorded journal panic fields are retained under
`_build/qa/live-hang-20261113/` as `threads-hung.txt`,
`threads-new-instance.txt` and `journal-panic.json`. The latter is a structured
transcription of the successful journal query, not a raw journal export.

A new software UI regression reproduced the same Slint panic by removing a Colors
rule while its real tooltip remained open. Slint instantiates popup children while
holding its popup-list borrow; destroying the row then tries to close that tooltip
through a conflicting borrow. Colors and keyboard-editor mutation callbacks now
dismiss tooltips before taking the presenter borrow and replacing row models.
The regression also covers keyboard selection with a live command-row tooltip.

Application startup now catches unwinding around the window attempt, explicitly
stops and awaits the image owner, consumes the runtime through bounded shutdown,
and resumes the original panic. It preserves normal cleanup and renderer-startup
retry behavior; an application panic does not trigger a software-renderer retry.
This addresses the observed idle-owner shutdown wait. It does not establish the
original UI callback or qualify arbitrary active decoder failures.

## Shortcut and function-bar follow-up

Options → Keyboard now offers an embedded **Replace shortcut** dialog when an
otherwise valid chord overlaps another command. The dialog identifies the owners
and their scopes. Replacement validates and applies one preference-map update,
preserving other aliases and wholly disjoint contexts. Options OK persists it;
Options Cancel rolls it back. Cancel is initially focused; Cancel, Escape and
default Enter leave bindings intact. Native editing reservations and alias limits
remain enforced. Confirmation uses an immutable popup-occurrence token plus the
selected action, candidate, editor revision and binding-map identity; stale
callbacks cannot affect a newer prompt.

The bottom function bar hides Connect/Disconnect actions when the platform lacks
network-drive dialogs, including Linux. Visible buttons fill the available space.
This follows the effective action after remapping, preserving Ctrl+F11 Zoom and
Ctrl+F12 Filter. A pointer press freezes action, pane and geometry through release,
including presses on Jobs while modifier layers change.

The final Debug test build and serial runner are recorded in
`_build/qa/live-hang-20261113/final-test-build.jsonl` and
`final-tests/summary.json`: **68 passed, 0 failed, 0 ignored**, all selected cases
executed. This is a targeted result, not the full workspace suite.

| Boundary | Passed |
| --- | ---: |
| Real tooltip retirement during Colors row removal and keyboard selection | 2 |
| Keyboard settings, replacement transaction, cancellation, stale tokens/widgets and rollback | 32 |
| Colors priority, picker, picker-popup and preference-validation integration | 11 |
| Function-bar actions, capture/geometry, small windows and shared control/dialog layout | 14 |
| Tooltip placement/interaction | 3 |
| Panic cleanup with a real idle image owner and existing spare-ordering contract | 6 |

Three unchanged presenter-level keyboard tests also passed in the earlier
`integration-tests/summary.json`; they are separate from the 68 final checks.
The pre-fix tooltip reproduction, retained-widget failure and fixture failures are
retained beside the final receipts. The rendered shortcut dialog was visually
reviewed at `final-tests/001-midnight-ui-test-keyboard_options/render/keyboard-shortcut-replacement.png`.
These software UI tests qualify their explicit interactions; they do not supply
the missing original native panic backtrace.

The final native smoke exposed an older installed native-filesystem library:
the app still displayed internal mount paths despite the committed filtering
source. The plugin was rebuilt in Debug, its **8 mount tests passed**, and the
new library was installed with an atomic replacement. The old file and its hash
are retained in `native-plugin-before-refresh.so` and `native-plugin-stage.json`.
No running process's mapped library was modified in place. Together with the
68 final checks, this follow-up has **76 passing targeted tests**.

`final-debug-build.jsonl` records the successful Debug ME/elevation-helper build;
`native-plugin-debug-build.jsonl` records the independent Debug plugin build.
The refreshed installation passed native graphics startup and automatic shutdown
(exit 0) using in-memory settings and owned empty directories. The final
`smoke/debug-startup.png` was visually reviewed: only the useful root drive is
listed on this machine, and the footer shows F1–F10 plus Jobs. The earlier smoke
and screenshot remain under `*-before-plugin-refresh.*`. The existing WGPU
discrete-GPU warning persists; this run makes no performance claim about it.
`build-result.json` records artifact hashes and the unchanged 3,093-input source
snapshot. No Release build ran, and neither existing user ME process was restarted.

## Options navigation font scaling

The later sidebar report reproduced with the actual selected **Custom Commands**
caption: at font 12 it required 130 px but received 112 px. The old sidebar width
used a character-count estimate that was too small. `OptionsBody` now measures all
topic titles with the selected weight (600), inherits the same font family and
size, and reserves list padding, text insets and the Fluent scrollbar overlay.
The sidebar and Colors content-width calculation use the same measured width.
Measurements remain independent of the visible sidebar and its viewport; the
existing narrow-window topic dropdown remains available.

The focused regression failed before the change and passed afterward. It checks
every selected caption, width stability across selection, font sizes 12/16/24,
touch spacing, a monospace font, vertical scrolling and the font-32 dropdown.
The font-12 and font-24 rendered snapshots were visually reviewed.
**51 targeted tests passed, 0 failed, 0 ignored**: `options_reflow` (9),
`options_footer` (3), `color_priority_list` (6), `keyboard_options` (32), and
`options` (1). These are software UI checks, not a full-workspace result.
Receipts and renders are under `_build/qa/options-navigation-20261113/`;
`red-tests/` retains the original failure and `green-tests/summary.json` records
the complete serial result. No dependencies changed.
`debug-build.jsonl` records the successful Debug executable build; the separate
native startup/automatic-shutdown smoke exited 0 with in-memory settings and
owned empty directories. `build-result.json` identifies the executable and the
unchanged source snapshot. Existing ME instances were not restarted.

## Archive preview option

Options → Previews now includes **Allow previews inside archives**, checked by
default for both new and legacy preferences. It follows the master preview
switch while retaining its independent value. Disabling it immediately hides
retained archive images and rejects late archive frames; the next normal 100 ms
scheduling pulse removes the targets through existing backend cancellation.
Native thumbnails and command artwork retain their paths. Network archives still
require the independent network opt-in and actual-container storage checks.
Source authority, archive reads and decoding remain in their existing workers.

Options edits preview immediately; OK saves, Cancel restores the baseline and
Reset previews the enabled default. Per-field edit tracking preserves this choice
through delayed settings loading without overwriting the other preview switches.
The strict settings codec adds schema 31 with `previews.archives`. Versions 1–30
default to true and reject that newer field. Explicit false writes schema 31;
the enabled default preserves prior conditional encodings. Schema 31 retains all
earlier required fields and rejects missing, null, duplicate and mistyped values.
No dependencies or public plugin protocols changed.

The staged pre-implementation tests produced two expected codec failures, four
policy/session failures and the missing-checkbox failure. The final serial run
records **214 passed, 0 failed, 0 ignored**, with every selected case executed:

| Boundary | Passed |
| --- | ---: |
| Settings unit tests and archive-preview, archive-association, editor and filter codec integration | 63 |
| Actual checkbox, preview size, Options transactions, keyboard and responsive/footer UI | 55 |
| Presenter preferences/loading, archive policy, retained images and row refresh | 82 |
| Application thumbnail scheduling, source tokens, captures and delivery | 14 |

The pointer-driven checkbox test retains a real 7×3 Slint image from a fixture
archive whose source methods reject UI I/O. It verifies immediate hiding,
late-frame rejection, Cancel, master-switch independence, persisted false and
Reset/Cancel rollback, including toggle → Reset in the same popup. A test sequencing failure was corrected by drawing the
save-acknowledged popup closure before reopening Options; production code did not
change for that fixture correction. Independent review subsequently exposed a
separate live Reset defect: clicking a one-way checkbox binding left its visual
state detached from the restored preference. A same-popup regression reproduced
the mismatch. The new archive flag now uses two-way bindings through MainWindow,
OptionsBody, ThumbnailOptions and the checkbox; the final 214-test run includes
that passing regression. The checked-default settings render was visually
reviewed. These are targeted software-UI and host-boundary tests, not installed
archive-format extraction or full-workspace qualification.

Receipts are under `_build/qa/archive-preview-option-20261113/`:
`red-build-complete.jsonl`, the three `red-*` runners,
`final-reset-build.jsonl`, `final-reset-tests/summary.json` and
`final-debug-build.jsonl`. The initial `green-tests/` fixture failure, intermediate
`final-tests/` pass and subsequent `live-reset-red/` failure remain separate from
the final passing run. The final Debug executable also passed native startup and
automatic shutdown with in-memory settings; existing user instances were left
untouched. Artifact hashes and the unchanged source snapshot are recorded in
`build-result.json`. Formatting, whitespace and the 100,000-byte handwritten
source limit passed. No Release build ran.

## Panel row alignment and notification cadence

The reported folder was empty when inspected, and the user could not establish
whether the screenshot came from the latest Debug executable. Synthetic ordinary
database/journal names matched the screenshot without reproducing the shift,
including metadata updates and native window resizing. The original filename
bytes and exact trigger therefore remain unconfirmed.

A trailing LF in a synthetic filename did reproduce the visual defect in both
software and native Winit/femtovg-wgpu rendering. Slint's `no-wrap` still honors
mandatory paragraph breaks; an invisible final paragraph changes vertical
centering inside the fixed-height row. In the native reproduction the ordinary
filename painted at row-relative y=5–15, while the LF filename painted at y=0–7
and was clipped. FileEntry now explicitly limits non-thumbnail filename and
size/date/time labels to one line. Thumbnail labels retain their multiline
layout, and raw names/operation identities are unchanged. The pixel regression
checks actual painted glyph bands rather than relying only on item rectangles.

Backend flat and tree watches now coalesce native changes after **150 ms** of
quiet, with a **500 ms** ceiling during continuous activity (previously 1 s / 3 s).
The 15 s metadata-poll fallback, bounded queues, existing workers, mutation guards
and single-flight refresh handling remain. These intervals govern when a refresh
becomes eligible; backend I/O and backpressure can add delay. Two deterministic
tests obtain the real production timing policy and exercise burst coalescing and
continuous-event deadlines; both failed with the old values and pass now.

Final targeted Debug results: **55 passed, 0 failed, 0 ignored**:

| Boundary | Passed |
| --- | ---: |
| Watch timing, flat/tree refresh and watch lifetime | 37 |
| Row geometry and actual filename pixels | 9 |
| Panel scroll and scroll-cost contracts | 5 |
| Application visible-sync and wake-gate integration | 4 |

Receipts are under `_build/qa/panel-rows-watch-20261113/`.
`watch-green-build.jsonl` / `watch-green-tests/summary.json` and
`ui-green-build.jsonl` / `ui-green-tests/summary.json` record the selected build and
test results. The software pixel failure is retained in `mandatory-breaks-red/`;
the native pre-fix LF failure is in `native-lf-red.log` and `native-lf-red/`.
The ordinary native pre-fix control passed, as recorded in `native-ascii.log`.

`final-debug-build.jsonl` records the successful Debug ME and normal native-probe
build. Three serial native probes (`--case=ascii`, `--case=lf`, `--case=cr`)
each completed all four steps: initial rendering, metadata update and two window
width changes. All eight sampled bands per probe are y=5–15; all probes exit 0.
`native-green-results.json` checks completion as well as process status, so an
early manual window closure cannot count as a successful run. The before/after
LF crops were visually reviewed. `native-smoke-result.json` records the final
Debug app's native startup and automatic shutdown (exit 0), using owned empty
directories and in-memory settings. The startup snapshot was visually reviewed.
The existing discrete-GPU WGPU warning remains outside this fix's scope.

The reproducible commands, artifact identities, source-drift check and receipt
paths are retained in `build-result.json`. This is targeted row/watch and native
startup evidence, not full-workspace or original-folder qualification. Existing
ME instances were left untouched; no dependencies changed and no Release build
ran.
