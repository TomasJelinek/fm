# Midnight Explorer: operations, panel modes and branding

This record covers the working-tree extension after `0961c96835505aa5a948be217607613db722643c`.
Earlier verification records retain the former FM command/crate names as historical
evidence. Current executable/package: `midnight-explorer`; internal crates use
`midnight-*`. Legacy settings constants and the existing Git remote remain intact.

## Recorded focused verification before the final renamed build

- Windows native provider: 70 unit, 4 ACL and 15 mutation integration tests passed;
  strict all-target Clippy passed. Three explicitly invoked Shell fixtures passed:
  Unicode recycle/restore, long-path fail-closed behavior, and junction recycle/
  restore preserving a readable target sentinel. No user Trash contents were purged.
- Linux provider: isolated native mutation/create, locale and settings migration
  tests passed. The Trash infrastructure regression first reproduced an incorrect
  successful recycle of `info`, then all nine related Trash tests passed with the
  identity guard for both `files` and `info`; strict provider Clippy passed.
- Engine: seven mutation tests passed, including confirmation idempotence, serial
  queue/cancellation, retained source identity after navigation, partial failure
  reporting, bounded jobs and cleanup retaining the job budget until handles close.
  The stopped-event-consumer network regression reproduced unbounded rejection
  tasks; final verification also covers accepted native calls retaining their permit
  through event delivery.
- Tab/session restoration and middle-click tests passed, including early user edits,
  explicit CLI overrides and queue pressure. Settings migration passed 8 Windows
  and 7 Linux tests without modifying legacy files.
- Initial panel modes passed 244 UI library tests plus panel-scroll and mode-picker
  integration. A 10,000-entry model checks bounded viewport reads, retained cursor,
  source-index mapping and metadata updates. Subsequent review added thumbnail
  prefix-highlight and wheel-notch regressions; final results belong below.
- Native detailed scrolling passed rendered-frame checks: 7, 8, 7 and 7 moving
  frames per notch, first step 14.93 px with 22 px rows, idle row alignment and
  ordinary/selection/theme pixel checks. See [measurement method](../NATIVE_SCROLL_VERIFICATION.md).

Independent review identified and prompted fixes for Trash infrastructure identity,
unnecessary Windows content-read permissions, bounded network feedback and missing
thumbnail prefix highlighting. Native Shell path-race and cancellation limitations
remain explicit in the platform safety document; no stronger guarantees are claimed.

## Final renamed snapshot

The empty-file extension passed eight Windows native tests and six Linux native
tests after recording their missing-capability failures. Combined with F7, all
eleven Linux creation tests passed. Both providers passed strict all-target Clippy.
Engine creation tests reproduced missing integration, then all nine mutation
queue tests and strict engine Clippy passed. UI tests reproduced missing Shift+F4
routing and clipping of the mode picker in a narrow Touch panel. Independent
review additionally found destination focus/reveal and rejected-submission focus
bugs; those regressions now pass. A ninth Windows native test additionally confirms
case-variant collisions agree with the directory's ordinary Windows lookup rules.

Windows workspace tests passed: **633 passed, 8 explicitly ignored**, followed by
the additional case-collision test. The ignored items comprise two optional CPU/
render benchmarks, three subprocess fixtures exercised by their parent tests,
and the three Shell recycling fixtures separately invoked above. All 247 Windows
UI unit tests passed. Strict Windows workspace/all-target Clippy passed; focused
final checks also cover the equivalent style-only let-chain and assertion cleanup
(469 passed, 6 ignored). Linux backend tests passed: **317 passed, 1 optional
benchmark ignored**. The full Linux workspace then passed **607 tests, 2 optional
benchmarks ignored**, including all 247 UI unit tests. Stale Options-coordinate
fixtures were replaced with accessible button clicks; original feature assertions
were retained. Strict Linux workspace/all-target Clippy also passed after replacing
one equivalent cloned singleton-slice assertion in the Unix-only test fixture.

All three native mode checks and the seeded mode screenshot passed; see
[native scroll verification](../NATIVE_SCROLL_VERIFICATION.md). The optimized
Windows release built successfully: **30,022,144 bytes** for `midnight-explorer.exe`.
Its 24 native creation/mutation tests and 9 engine-operation tests passed under
the release profile. The actual EXE passed filesystem and native UI smoke checks
with exit code 0; `windows-release-ui.png` was visually inspected with real native
drive/file icons, both live panels, toolbar controls and the compact footer.
Linux release checks passed: **34 native mutation/watch tests** and **9 engine
operation tests**. The Windows engine run was repeated with the correct
`operations::tests` module filter after the initial filter selected zero tests;
`windows-release-queue-green.log` records all nine executed tests. Linux used the
corrected filter too. A Linux optimized GUI binary has not been built or exercised
in this verification record.
Generated products/logs remain in `_build`.
The source-size guard currently checks 268 handwritten files, all <=100000 bytes.

The first renamed mode snapshot generated 13,498,972 formatted Rust bytes for Slint,
compared with 12,282,479 in the preceding detailed-layout local build. This is
generated bindings/layout machinery, not handwritten Rust or executable size.
The historical 8,704,058-byte shared-button checkpoint predates later functionality;
it must not be presented as the current output. Panel cells and renderer controls
are shared/virtualized; viewer plugins will share one host renderer rather than
embedding their own Slint builds. The final Windows release-generated `main.rs`
is **13,660,844 bytes**; the executable size is recorded above. Generated source
text is not an executable-size measurement. Final binary size and UI measurements
determine the release impact.

All logs and captures named here are local QA artifacts under `_build/qa/mutations`
or `_build/qa/native-scroll`, not committed binaries. To make room for the release,
only the validated `_build/debug/incremental` cache was removed (43,718,742,088
generated bytes); sources, executables and test evidence were retained. The build
duration reported by Cargo spans an overnight interruption and is not a performance
benchmark.
