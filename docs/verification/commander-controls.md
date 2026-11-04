# Commander controls and automatic refresh verification

Date: 2026-11-04. Windows x64 and WSL Ubuntu 24.04 use the pinned Rust toolchain.

## Behavior

- Full F1–F10 bar, external-terminal command editor, safely quoted standalone
  Ctrl+[ / Ctrl+] path insertion, Alt+Enter properties, parent row and native icons.
- Independent drive/special-location bars; Windows Shift+drive-letter shortcuts.
- Active-pane accent/frame; native Slint short wheel animation and touch inertia.
- Cancellable Ctrl+Shift+S directory sizing, progressive/partial states, cached
  backend sorting with ascending/descending column toggles and stable identities.
- Native nonrecursive watches per OS, 1s quiet/3s maximum debounce, and 15s exact
  metadata polling for lost notifications. Same-location refresh preserves rows,
  cursor, selection, viewport and command text/caret/focus.
- All native, filesystem, decoding, traversal and large-data cleanup stays on
  bounded backend workers. Copy/move/delete and their verification queue remain
  the next milestone; unavailable actions are disabled.

## Targeted evidence before final workspace checks

- Core literal quoting: RED3 → GREEN3, plus raw-invalid-Unicode rejection tests.
- PowerShell's actual parser round-tripped Czech/CJK, ASCII/smart apostrophes and
  dollar/semicolon syntax in a literal; it did not execute the parsed input.
- Folder traversal: 13 tests; orchestration: 11 tests including failed rollback,
  an unaccepted intermediate listing, stale/duplicate requests and measured sorts.
- Watch controller: 14 tests covering debounce cap, independent panes, lost
  notifications/polling, metadata recovery, blocked probes, queue saturation,
  target changes, native failure, callback cleanup and shutdown.
- Real Windows native watch → engine → accepted-pane hint → Unicode file reload
  passed, taking approximately 1 second after the file mutation.
- UI: 42 unit tests and actual key/render/scroll integration passed. This includes
  unsafe quoted-context rejection, Unicode caret/selection, native-key precedence,
  disabled actions, wheel continuation and same-location refresh without jumps.
- Native suites: Windows 8 tests; Linux 13 tests, including actual filesystem
  mutations and Windows paths beyond 260 UTF16 units. Linux FIFO regression
  observed RED then GREEN after nonblocking opened-descriptor validation.
- A fresh independent review drove fixes for metadata recovery, snapshot rollback,
  insertion context, FIFO reads and indexed UI properties. Watch review found no
  additional material blocker after the metadata-recovery correction.

Raw evidence: `.tools/controls-*.log`, `.tools/watch-*.log` and
`.superpowers/sdd/commander-controls/`. Final workspace results are recorded below
after commands complete; test configuration alone is not evidence of execution.

## Limitations

Later authorized extensions add headerless Blue/Dark/Light appearance,
Desktop/Touch controls, persistent preferences, terminal selection and storage-aware
folder batching. Fresh targeted evidence is52 UI unit +2 actual input/render
tests,9 preferences-worker tests,24 folder tests,6 common storage/codec tests,
24 Windows native tests and22 Linux native tests. Final whole-workspace counts
below supersede the earlier targeted-control counts.

Viewer/VFS/native-provider migration and automatic reusable sudo routing are
design follow-ups, not enabled implementations. Overwrite dialogs belong to
the planned safe mutation milestone. Windows terminal startup may reject a long
cwd even with an embedded longPathAware manifest; the full path is kept and the
OS error is reported, never a different working directory. Native new consoles
have their own std handles even when FM diagnostics are redirected.

Linux native icons currently support bounded installed PNG themes; missing assets
use generated RGBA fallback. Linux and Windows multi-selection properties use
read-only metadata when native integration is unavailable. Slint command caret
access is tied to the pinned 1.18.1 byte-offset API. Directory sizes are live
logical-length estimates, skip explicit links and can be partial; they do not
constitute atomic filesystem snapshots or future destructive-traversal safety.

notify 8.2.1 can lose some Windows error/overflow signals. Periodic polling remains
enabled even when native registration succeeds. Platform/provider failures can
delay refresh; no real-time delivery guarantee is asserted. macOS is future work
and must provide its own platform adapter. Dependency advisory warnings and the
software-renderer's earlier Unicode limitations remain documented.

## Final workspace results

All final commands completed successfully against the implemented controls:

| Check | Windows x64 | Ubuntu24.04 / WSL |
| --- | --- | --- |
| Workspace tests |189 passed |183 passed |
| Strict Clippy, all targets |Pass |Pass |
| App build |Pass |Pass |
| Handwritten source-size guard |85 files, all <=100000 bytes |85 files, all <=100000 bytes |
| Engine executable smoke |Pass, explicit workspace/Unicode fixtures |Pass |
| Native window and screenshot |Pass, real user desktop |Headless actual-render integration; no native desktop screenshot claimed |

Formatting passed. Two Windows tests are subprocess-only ignored fixtures;
their passing parent tests execute them explicitly in hidden native consoles.
The suite verifies new-console stdio isolation and exit259 handling. Actual
PowerShell parsing and Cmd Unicode/metacharacter no-op probes also passed.

Final native screenshot: [Windows controls](../screenshots/commander-controls-final-windows.png).
The updated executable was launched under the real user account, PID73472,
with visible/responding `FM — File Manager` window. Foreground activation was
refused by Windows, so no claim that it is in front of the current app is made.
Evidence: `.superpowers/sdd/commander-controls/native-final-window.json`.

Cargo audit of the actual final lockfile against the locally fetched advisory DB
reported0 matching known vulnerabilities and5 unmaintained transitive warnings.
This is not proof of absence of unknown vulnerabilities. Detailed audit JSON is
`.superpowers/sdd/commander-controls/cargo-audit.json`.

Linux native/tests and Windows source fixtures do not prove a separate Mint
installation, macOS support, destructive file operations, automatic sudo reuse,
or an implemented viewer/VFS. These remain declared follow-up work.
