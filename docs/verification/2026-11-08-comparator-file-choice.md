# Comparator File A / File B fallback — 2026-11-08

Implemented the manual fallback described in [the comparison contract](../developer/FILE_COMPARISON.md).
Automatic panel pairing remains the first choice. Missing/ambiguous inputs now open
the existing native comparator window with two editable fields and native Browse.

## Evidence

- Red: `unresolved_pair_keeps_session_available_for_manual_file_selection` failed
  against the previous implementation because the worker terminated on an empty pair.
- Windows comparator: 22 engine tests, 1 raw UTF-16 presentation test and 2 real
  Slint integration tests pass. Tests cover typed paths, Enter, Browse cancellation
  and selection, native owner forwarding, busy duplicate suppression, invalid paths,
  retry, Escape with an edit focused and the existing comparison controls.
- Windows main UI: all 521 unit tests and 9 shared toolbar tests pass (555 tests
  total including the comparator). The shared toolbar suite covers opening
  comparison without any panel target, active-panel dispatch and existing action gates.
- Linux: 43 tests pass. Comparator tests also cover raw non-UTF-8 path preservation; presenter
  pairing tests (9) and shared toolbar tests (9) pass in WSL.
- Strict comparator Clippy passes on Windows and Linux. Scoped rustfmt/diff checks
  and the 100 KB limit pass (1,262 source files). No new third-party dependency or
  plugin ABI change.
- Visual inspection: rendered File A/B form at 1000×700 and 320×200. Small layout
  scrolls its fields/message while keeping Compare and Cancel reachable.
- Fresh independent review found one issue: an ambiguous active selection discarded
  an explicitly selected opposite file. Fixed, with regression coverage for both
  active panels. Follow-up review reported no outstanding findings.

Artifacts are under `_build/qa/comparator-choice/`. Build/profile, final lint and
startup results are recorded there; build and screenshot artifacts are not source.

Windows debug build: `_build/debug/me.exe`, 84,874,240 bytes, SHA-256
`a46e4394cbcaabb84a641a7622b6b89f2f8106e5d7548d34b159d34e5bf04aa5`.
The app build succeeds with the existing 34 warnings from unfinished tree-engine
code. The comparator crates pass Clippy with warnings denied. Release was not rebuilt.
The matching protected plugin runtime was registered at
`C:\Users\tjelinek\MidnightExplorer-20261108-133357-db6052ee`. Native startup from
`_build/debug/me.exe --ui-smoke-test` exited 0, produced the inspected `startup.png`
and empty stderr, and left user preferences unchanged (SHA-256
`bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`).

## Revision-guard diagnostic

Two intermediate Windows runs detected a source revision change on freshly written
test fixtures during rendering (including an existing automatic-pair test). The
comparison correctly rejected the result. A separate 20-file/5-second capture/read
probe and five subsequent diagnostic suite runs did not reproduce the change; its
cause remains unproven. Temporary instrumentation was removed. Final suites run with
the original identity checks, without retries or weakened revision validation. This
record preserves the failed-run evidence rather than claiming the transient was fixed.

Native picker behavior uses the existing platform implementation. Headless picker
integration tests inject a deterministic service; they do not exercise an interactive
Windows Shell or Linux portal dialog. macOS remains outside current build coverage.
