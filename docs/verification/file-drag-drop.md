# File drag/drop verification

This supplements the [design](../superpowers/specs/2026-11-06-file-drag-drop-design.md)
and [security contract](../../SECURITY.md#file-drag-and-drop). Native protocol tests
are distinct from UI gesture tests and from verified-copy integrity tests.

The 2026-11-06 results below describe the original Copy-only transport. Current
folder targets and Windows outgoing Move evidence are recorded in the
[2026-11-10 qualification](#2026-11-10-folder-targets-and-windows-shell-move).

## Reproducing native Linux checks

Install the README build prerequisites plus `xvfb` and `xauth`. Run from the repo
root on Linux with the pinned toolchain:

```sh
xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" \
    env MIDNIGHT_DND_TEST_ISOLATED=1 \
    cargo test -p midnight-platform-linux --lib --locked \
    file_drag_drop:: -- --ignored --test-threads=1
```

These tests deliberately use XTEST input on their private X server. Do not set
`MIDNIGHT_DND_TEST_ISOLATED=1` on your working desktop display. The repository's
Linux CI includes this isolated invocation; ordinary `cargo test --workspace`
alone skips these native scenarios. Xvfb is a test dependency, not an application
runtime requirement.

Checks exercise verified proxy installation/restoration and removal of the host's
legacy XDND advertisement; real incoming and outgoing peers; raw filename bytes;
bounded incremental transfers; stale destinations, rejected admission and changed
selection ownership; Escape; delayed release of the UI connection's pointer grab;
and Copy negotiation when the source proposes Move but explicitly permits Copy.
The self-proxy scenario transfers 1,000 paths through INCR.

The 2026-11-06 local platform run passed **117 tests** (106 library, 3 metadata,
8 transfer), including all **10 native drag/drop scenarios**, with zero skipped or
failed tests. Strict platform Clippy also passed. This run used isolated Xvfb
`:97`, Ubuntu 24.04.4/WSL2 and a local Czech locale fixture. Logs:
`_build/qa/drag-drop/linux-package-final.log` and `linux-clippy-final.log`.
RED/GREEN evidence also records proxy-advertisement, Copy-negotiation and
cross-connection pointer-handoff regressions. The full workspace run is recorded
separately once integration completes.

## Reproducing native Windows checks

Normal native unit tests include real COM file objects and a production session
registered against a window owned by another STA:

```powershell
cargo test -p midnight-platform-windows --lib --locked file_drag_drop::
```

The registration test marshals the actual IDropTarget and verifies that payload
admission occurs on the backend STA, then checks safe revocation/re-registration.
It does not move the mouse, change clipboard ownership or copy files. Other tests
cover raw unpaired UTF-16, malformed/oversized payloads, target coordinates,
refusal/admission effects, lifetime cancellation and swapped primary mouse-button
mapping without changing the machine setting.

An opt-in modal-loop smoke test briefly creates a tiny non-activating window on
an interactive test desktop. It invokes the real OLE loop with a test input source
delegating continuation policy to the production source; the target records
admission only. It injects no physical input and performs no filesystem operation:

```powershell
cargo test -p midnight-platform-windows --lib --locked `
    native_modal_copy_and_idle_cancellation -- --ignored --nocapture
```

The final native source harness passed **7 ordinary tests**. The explicit desktop
modal smoke also passed (0.64 seconds). The initial idle-cancellation watchdog
failure exposed a real OLE idle-loop problem: a timer alone does not reevaluate
the continuation callback. Cancellation now posts a wake only to the worker's
owned helper window; teardown drains that helper's pending wake before another
drag can start. No physical keyboard input is injected. The swapped-button and
stale-wake regressions were observed failing before their fixes and passing after.
Logs: `_build/qa/dnd/native-harness-green.log`, `windows-modal-green.log`,
`cancel-wake-red.log`, and `primary-input-{red,green}.log`.

## UI, queue and application integration

The first Windows release UI run passed **303 checks**: 299 library tests, a gesture
test across Detailed/Brief/Thumbnails, the full mouse regression and two compact
header/long-path checks. A later focused run passed four installed-bridge checks
plus nine Commander edge checks: internal fallback over a folder row uses the
containing panel, release/Escape/navigation prevent late revival, swapping/zoom/
modal UI update regions, and touch/right-button movement does not start dragging.
The highlighted full-panel copy target was visually reviewed.

Independent review identified pending-admission ordering, cancellation-before-
enqueue, transient-native-failure retry and swapped primary-button regressions.
The subsequent release run passed **306 checks**: 300 library tests, four installed
drag tests and two public queue regressions. Later clipboard copies cannot
overtake an acknowledged pending drop, and cancellation rejects its reserved token
before a backend job exists. Transient native failure permits a fresh native
gesture. The engine source harness passed **151 tests**, with one intentional
skip, including eight drag/drop and shared-copy-budget regressions. A bounded
cancelled-history regression reproduced 20 terminal jobs against the limit of 16;
the final history fix passes in both OS integration runs. The final Windows
release UI run passed **310 tests**: 301 library, four installed drag checks,
three public queue/history regressions, one footer and one eight-theme test.
An additional operation-ID exhaustion regression verifies rejection without
creating a job, wrapping the counter or admitting a replayed token. The current
Linux library/affected integration run passed **606 tests**, with 15 intentional
skips; the separate native protocol checks above cover the opt-in transport tests.
Independent source review found no remaining code issue.

The exact final Windows release application passed the filesystem and default
Direct3D UI smokes, including snapshot review and unchanged native preferences.
Its embedded moon icon's nine resources match the source ICO frame hashes.
Final integrated evidence and the broad-run contrast correction are recorded in
[the integration checkpoint](2026-11-06-themes-and-commander-tools.md#final-integration-regression-checks).
Logs include `_build/qa/clean-rebuild/windows-final-ui-tests.log` and
`linux-final-affected-tests.log`.

The fresh release build, graphics backend diagnosis and startup memory observation
are recorded in [the rebuild checkpoint](2026-11-06-themes-and-commander-tools.md#clean-release-and-requirements-checkpoint).
At that historical checkpoint, native Wayland external transport, legacy X11
clients without proxy support, Move/Link operations and another application's
copy integrity were outside the implemented transport contract.

## 2026-11-10 folder targets and Windows Shell Move

Backend/native and integrated component qualification is complete for this slice.
The final Windows debug application was built, deployed and passed native startup/
UI smoke tests; [the artifact record](2026-11-10-staging-cleanup.md#debug-executable-and-startup)
contains its exact hash and installation paths. This does not replace the separate
final-app-to-Explorer physical-input qualification described below.

Twelve integrated drag tests passed across `drag_drop`, `drag_integration` and
`drag_ordering`, including exact folder targets in all panel layouts, stale
capture/completion handling and a stationary pointer after scrolling. The same
run passed nine Copy-error, nine transactional Options and three Jobs integration
tests (33 total). The final UI library gate passed 786 tests. These component and
backend checks do not claim an unperformed end-to-end desktop gesture.

Windows outgoing offers now permit Copy|Move. Actual Shell `IDropTarget` tests
passed two opt-in cases containing four real transfers, each with a Unicode file
and a nested directory:

- Same actual volume: default Move and Ctrl-Copy.
- Distinct D: and C: volumes: default Copy and Shift-Move.

Every transfer checks destination payloads and source removal/preservation.
Fixtures are uniquely owned temporary directories; the C: fixture is removed
after the test. No existing user files or clipboard contents are used. The Shell
target, rather than this application, performs external moves. On this Windows
build, optimized Move returned `DROPEFFECT_NONE` with performed/logical Move
feedback; Copy returned Copy for all three values. Completion handling must not
interpret the returned effect alone as proof of filesystem state.

Three ordinary feedback tests pass: bounded DWORD round-trip and invalid-value
refusal, `STGMEDIUM` ownership for successful/failed `SetData`, and retained source
bytes after unoptimized Move feedback. A generic target that requests source
deletion receives `SourceRetained`; absent/inconsistent completion is
`Unconfirmed`. Positive results are receiver reports, not independently verified
external operations. Feedback handling performs no source deletion.

The real input-thread to backend-STA test also passes success/cancel/success. It
uses physical input limited to owned windows, restores cursor/foreground state,
and observes production `QueryContinueDrag` state without manufacturing a held
button. The original failure produced no continuation callbacks; scoped input
queue attachment fixes this boundary. This is separate from an end-to-end drag
from the final Slint application into a running Explorer window, which remains
part of pending integration qualification.

Final Windows platform library: **162 passed, 11 opt-in tests ignored**.
The two Shell cases and one real handoff case were then explicitly run and passed.
Strict `midnight-platform-windows --all-targets` Clippy with `-D warnings` passed.
The metadata-only device-hint regression also passes: it observes actual volume
identity without acquiring Delete/content-read authority, even while a no-delete
sharing pin refuses ordinary Move preparation.

Evidence under `_build/qa/native-drag/`: `outgoing-red.log`, `shell-red.log`,
`device-red.log`, `outgoing-green.log`, `platform-green.log`,
`shell-final-green.log`, `handoff-final-green.log`, and `clippy-final.log`.
The initial device fixture's metadata-only sharing assumption was corrected
before its behavioral RED; the final RED is missing device identity, not a
fixture assertion failure.

Internal drops use backend volume evidence: same-volume Move, otherwise Copy,
with unknown/mixed volumes conservatively Copy; Ctrl/Shift force Copy/Move.
Same-volume native rename reads no content or hashes. Copied data uses full
SHA-256 by default or explicitly selected size plus first/middle/last 64 KiB
samples (whole files up to 192 KiB). Native incoming transport remains Copy-only
on both platforms. Linux outgoing X11/XWayland remains Copy-only; external native
Wayland and Link remain unsupported. No dependency or hash algorithm was added.

Protocol references: [Shell optimized moves](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios#handling-optimized-move-operations),
[logical performed effect](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard#cfstr_logicalperformeddropeffect),
and [`IDataObject::SetData` ownership](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nf-objidl-idataobject-setdata).

## 2026-11-13: File tiles target the current folder

The user confirmed that dragging `Pictures/<folder>` into the opposite Screenshots
pane was rejected specifically over image tiles. Both paths were ordinary owned
directories. The internal UI hit-test resolved a PNG tile to a row, then rejected
it because only directory rows could be destinations. The existing backend sibling
directory policy already allowed this transfer; no backend containment rule was
relaxed, and the user's folder was never moved or copied during verification.

Flat native panels now treat an ordinary file row as the current pane root after
the existing kind, virtual-identity, row-identity and navigation guards. Folder
rows retain exact folder targets. Parent rows, links, special entries, stale rows
and tree file/context occurrences retain their existing rejection rules. Preview
and release use the same resolved destination; pane-root feedback outlines the
panel with “into this panel”, without highlighting or naming the hovered file.

The new real pointer-dispatch regression uses a directory source and 36 PNG rows,
starting in Thumbnails and covering Detailed and Brief too. It reproduces the
original NotAllowed feedback before correction, then verifies the exact pane-root
preview, allowed Move feedback and final drop request. Empty thumbnail content
and current native identity have separate checks. Earlier synthetic mode loops
set window properties directly, which drag startup could reset from presenter
preferences. They now change modes through the installed callbacks and assert
that the mode survives startup/preparation; historical passes from those old
loops alone did not establish genuine thumbnail-mode coverage.

Final selected results: **66 passed, 0 failed, 0 ignored**:

| Boundary | Passed |
| --- | ---: |
| Pointer integration, all-layout file/folder targeting, navigation/cancellation and geometry | 13 |
| Drag interaction and accepted-drop ordering | 5 |
| Presenter flat/tree authority and drag lifecycle | 24 |
| Backend admission, device intent, stale previews and containment | 24 |

Receipts and rendered feedback are under `_build/qa/file-tile-drop-20261113/`:
`red-tests/`, `green-test-build.jsonl`, `green-tests/summary.json`,
`backend-tests/summary.json`, `debug-build.jsonl` and `build-result.json`.
The mode-2 allowed-feedback render was visually reviewed. The unchanged backend
test executable was selected from its prior successful Debug receipt, with hash
validation; no redundant backend compilation was needed.

The actual Debug application/elevation build and a separate native startup/
automatic-shutdown smoke passed with owned empty folders and in-memory settings.
Existing user processes were left running. These pointer tests use software UI
event dispatch, not a measured physical desktop gesture. No Release build or
dependency change ran, and no Windows native qualification is claimed here.

## 2026-11-21: Drags released within 400 ms

A drag from a panel that is released less than 400 ms after the mouse button press
that started it (`MIN_DRAG_DURATION` in `crates/midnight-ui/src/presenter/drag_drop.rs`)
is cancelled wherever it ends: nothing is copied or moved and no dialog opens, the
prepared drag is cancelled, selection and cursor stay, and the source panel's status
line says "Drag cancelled: released within 0.4 s" for about 4 seconds or until the
next drag. A release at 400 ms or later drops as before. The drag bridge
(`crates/midnight-ui/src/drag_bridge.rs`) hands a drag that has left the window to
the operating system, and so to other applications, only once the press is 400 ms
old; until then the window keeps the pointer, so a quicker release outside it is
cancelled the same way.

Run on Windows 11 (unoptimized `test` profile) on 21 November 2026, at commit
`c5c95186` with no local source changes:

```powershell
cargo test -p midnight-ui --lib --locked --offline -- drag_bridge presenter::tests::drag_drop
cargo test -p midnight-ui --test integration --locked --offline -- drag_integration::
```

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Library: 9 `drag_bridge` and 14 `presenter::tests::drag_drop` tests | 23 | 0 | 0 |
| Integration: `drag_integration::` | 14 | 0 | 0 |

Four of these tests check the 400 ms rule on a test clock, each comparing a release
1 ms before 400 ms with a drag that reaches 400 ms:

- `presenter::tests::drag_drop::a_drop_sooner_than_the_minimum_drag_duration_cancels_and_one_at_it_proceeds`:
  onto the other panel's folder row and its current folder, with automatic, Move
  and Copy intents. The quick release sends no drop and no native drag, cancels the
  prepared drag, keeps cursor and selection and notes it on the source panel only.
- `drag_bridge::tests::a_drop_sooner_than_the_minimum_drag_duration_after_its_press_is_cancelled`:
  the same through the window's pointer events; the note goes after 4 seconds.
- `drag_bridge::tests::other_applications_get_a_drag_only_once_it_has_lasted_the_minimum_duration`:
  a drag outside the window stays the window's until 400 ms; released before then,
  it is cancelled and noted; otherwise the next poll starts the hand-off to the
  operating system (which the test's software window cannot complete, so the bridge
  then cancels the drag without the note).
- `drag_integration::drops_sooner_than_the_minimum_drag_duration_on_the_other_panel_or_a_folder_are_cancelled`:
  the integrated main window, onto a folder row and onto the other panel's empty
  content.

Logs: `_build/qa/g-ui-drag-lib.log` and `_build/qa/g-ui-drag-integration.log`.
These tests dispatch software pointer events; no physical desktop gesture and no
drag into another application was performed.

## 2026-11-23: Dragging to and from other applications

The owner reported dragging outside the application as missing on Windows and Linux.
Before changing anything, the unmodified application (base `77a22d64`, unoptimized
build, `--ui-smoke-test --software` with in-memory settings and private folders) was
dragged with the real mouse on Windows 11 (build 26200, 200 % scaling) to and from a
private WinForms probe window that logs what OLE delivers. The probe and its driver
stay under `_build/dnd-probe/`; they are diagnostics, not product code. Linux could
not be run here: the findings below for Linux come from the source and the existing
Xvfb tests.

| Scenario | System | Before | Why | Now |
| --- | --- | --- | --- | --- |
| A file dragged out of a panel to another application's window | Windows | Worked, poorly: the target got CF_HDROP only, no Shell item list, no drag image | Our own data object carried one format; no drag image helper | Shell data object (Shell item list, file descriptors and the same CF_HDROP list), Shell drag image with the item count |
| The same, moved there quickly and held | Windows | The operating system took the drag only once the press was 0.4 s old; until then the target showed nothing | The drag bridge waited for 0.4 s and for the pointer to leave the window | The drag goes to the operating system as soon as it leaves the window; the target reacts at once |
| A quick flick out of the window, released within 0.4 s | Windows | Cancelled, the target never saw it | As above | Still cancelled with the note "Drag cancelled: released within 0.4 s"; the target sees the drag enter and leave, never a drop |
| Explorer windows and the desktop as targets | Windows | Same-volume Move, cross-volume Copy, Ctrl/Shift as Explorer decides (2026-11-10 Shell tests); not tried against live Explorer windows | — | Unchanged; Explorer now also gets its Shell item list. Not re-run against live Explorer windows (the owner's desktop) |
| Move to a target that moves the files itself (optimized Move) | Windows | Reported Moved; the source panel refreshes | — | Unchanged, now read from the Shell object; covered by the real-mouse test below |
| Files dropped in from a .NET program (WinForms, WPF) | Windows | Failed: "The dropped file list is malformed or not Unicode", nothing copied | Their DROPFILES header writes `fWide` as -1; the decoder accepted only 1 | Any nonzero BOOL is Unicode; the probe's file was copied into the panel. Clipboard paste from such programs shares the decoder |
| Files dropped in from Explorer and other native programs | Windows | Copied | — | Unchanged |
| Drop onto a folder row | Windows, Linux | Copied into the panel's current folder, not into the folder | Native drop targets are whole panels | Open: native targets are still whole panels (folder rows only for drags between panels) |
| Drop onto tabs, the path bar, drive buttons | Windows, Linux | No drop | By design: they are not drop targets | Unchanged |
| Rows of an archive or the Network view dragged | Windows, Linux | Nothing happened, without a reason | Such rows have no native file to hand over | The status line says "Items in an archive or the Network view cannot be dragged; copy archive items with F5." No virtual-file export |
| Drag out to Nautilus, Dolphin, a browser, another instance (X11, XWayland) | Linux | Copy-only XDND with `text/uri-list` (Xvfb peer tests) | — | Same transport; the drag starts on leaving the window and a release before 0.4 s sends XdndLeave instead of XdndDrop. Not run against real desktops here |
| Any external drag in or out of a native Wayland window | Linux | Refused: panel to panel only | No same-connection seat or data device | Implemented on winit's own connection (`wl_data_device`, `text/uri-list`, Copy). Compiled and linted for Linux; its unit tests and a compositor qualification have not run |

### Windows evidence

Commands, from the checkout root with `CARGO_TARGET_DIR` set to a private folder:

```powershell
cargo test -p midnight-platform-windows -p midnight-engine -p midnight-ui --lib --locked --offline --no-run
midnight_platform_windows-<hash>.exe file_drag_drop file_clipboard
midnight_platform_windows-<hash>.exe real_mouse_drag_out_reaches_a_foreign_target --ignored --nocapture --test-threads=1
midnight_platform_windows-<hash>.exe real_mouse_handoff_to_worker_sta --ignored --nocapture --test-threads=1
midnight_ui-<hash>.exe drag_bridge presenter::tests::drag_drop presenter::drag_drop
midnight_engine-<hash>.exe drag_drop
cargo test -p midnight-ui --test integration --locked --offline -- drag_
```

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Windows platform `file_drag_drop` and `file_clipboard` | 24 | 0 | 6 |
| Real-mouse drag out to a foreign OLE target (desktop lease) | 1 | 0 | 0 |
| Real-mouse handoff to the worker STA (existing, desktop lease) | 1 | 0 | 0 |
| UI library `drag_bridge` and presenter drag tests | 26 | 0 | 0 |
| Engine `drag_drop` | 43 | 0 | 0 |
| UI integration `drag_` | 33 | 0 | 0 |
| Acceptance `files_clipboard_drag`, `journeys_pointer_only` (run `20261123-212637`) | 2 | 0 | 0 |

Workspace Clippy (`--workspace --all-targets --features midnight-acceptance/acceptance
-- -D warnings`), `cargo fmt --all --check` and the source-size check pass.

The real-mouse test presses on an owned window, lets the production drop source run
the drag, moves onto a foreign target window on another STA and releases: with Copy
the target saw the Shell item list, the drag image and Copy|Move offered, read the
exact Unicode path and the outcome was Copied; with an optimized Move report it was
Moved; released before its earliest drop time the target saw the drag leave and no
drop, and the outcome was ReleasedTooSoon. Red before the fix: the .NET header test and
the bridge's handoff test failed on the old code (`_build/qa/red-hdrop.log`,
`red-bridge.log`), and the probe logged CF_HDROP alone, no drag image and a refused
.NET drop. With the fixes the application, dragged by the real mouse, gave the probe
`Shell IDList Array`, `DragImageBits`, `FileGroupDescriptorW` and the file; a release
0.24 s after the press showed the target the drag entering and leaving, no drop, and
the note on the source panel; a drop from the probe was copied.

A drop target helper (`IDropTargetHelper`, drag images and "Copy to" descriptions over
this window) was tried and removed: it deadlocked the existing marshalled-target test,
in which the window's thread waits in a COM call while the helper waits for that
window. A drag image from another program over this window is therefore whatever that
program draws itself.

### Linux

`cargo clippy --locked --offline --target x86_64-unknown-linux-gnu -p midnight-platform-linux
-p midnight-engine -p midnight-core --all-targets -- -D warnings` passes on the Windows
host. The UI crate's Linux adapter (`drag_input_linux.rs`) needs fontconfig to build for
Linux and was not compiled here. Owed on Linux: the platform library tests, including
the new `file_drag_drop::wayland::tests` and the isolated Xvfb XDND scenarios; a native
Wayland qualification (for example Weston's headless backend with a test client, then
GNOME and KDE) of drags in and out, Escape, a release within 0.4 s and a stale serial;
and drags to Nautilus, Dolphin and a browser under X11 and XWayland.
