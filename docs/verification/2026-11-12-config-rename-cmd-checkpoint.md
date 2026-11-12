# Configuration, native Rename, Cmd and cancellation checkpoint

Date: 12 November 2026. Baseline: `f0cf369`. This checkpoint covers four coordinated
changes, not completion of the entire implementation backlog.

## Implemented behavior

- Copy/Move/Delete/Recycle progress asks before cancellation from Escape, native
  or header X, and Cancel operation. Continue is the default. Dismissing the
  question preserves the existing running/paused state; only explicit confirmation
  cancels the exact job/session. Completed windows close normally. Jobs history
  remains separate. [Confirmation evidence](2026-11-12-progress-cancel-confirmation.md).
- Optional engine services use one-time vacancy admission before an owner is
  constructed/transferred. Duplicate configuration cannot synchronously destroy a
  displaced service on the caller. [Configuration evidence](2026-11-12-engine-service-configuration.md).
- Native inline-tree F2 Rename captures the focused acknowledged occurrence and
  retains existing preparation, confirmation and recoverable overwrite handling.
  Final capture ownership retires off the UI; shutdown also cancels retained F4
  editor captures. [Rename evidence](2026-11-12-native-tree-rename.md).
- Windows Custom Commands support literal single-line Cmd source with raw Unicode
  environment and separately encoded external-program argv companions. Native cwd
  identity is checked; unsupported reparsing/length/cwd cases remain explicit.
  [Cmd evidence](2026-11-12-cmd-custom-commands.md).

The combined ownership ledger has 308 payload slots and seven waiters. Rename adds
four slots under the existing retirement domain; service configuration adds none.
No plugin ABI, third-party dependency or additional cleanup thread changed.

## Final gates

Local evidence is under `_build/qa/config-rename-cmd-20261112/` unless stated.
Logs, generated binaries and screenshots are intentionally not committed.

| Gate | Result | Duration |
| --- | --- | --- |
| Windows full engine library/integrations | 1,331 passed, 12 ignored across 43 targets; library 1,000 passed/7 ignored, integrations 331 passed/5 ignored | 344.56 s including build; library execution 8.16 s |
| Windows affected UI/platform libraries and Jobs/native-operation integrations | 1,446 passed, 20 ignored across eight test executables | 132.785 s including build |
| Windows focused native progress windows | 19 passed | 0.79 s execution |
| Windows focused progress rendering | 3 passed | 0.35 s execution |
| Windows corrected real F2/F7 key routing | 3 passed | 2.12 s execution |
| Windows strict all-target Clippy | core, engine, UI, Windows platform, explorer; exit 0 | 224.148 s |
| Linux full engine library/integrations | 1,308 passed, 11 ignored; library 991 passed/7 ignored, integrations 317 passed/4 ignored across 42 integration targets | 293 s including build; library execution 14.66 s |
| Linux affected UI | 75 passed across tree presenter/F4, Cmd form, Rename, progress, Jobs and native-operation windows | Initial UI build 549 s; corrected native-window rerun 88 s |
| Linux strict all-target Clippy | core, engine, UI, Linux platform, explorer; exit 0 | 569 s |
| Workspace formatting | `cargo fmt --all -- --check`; exit 0 | Final recorded check |

Windows affected-library counts: elevation 7, Windows platform 183/12 ignored,
UI 914/4 ignored, input 29, viewer engine 187/4 ignored, viewer UI 115;
Jobs dialog 1 and native-operation windows 10. The three separately focused
Windows progress/Rename suites above are additional, not included in that 1,446.

Linux affected UI counts/timings: tree presenter/F4 31 (0.19 s), Cmd form 8
(0.13 s), Rename 3 (1.78 s), progress rendering 3 (1.73 s), Jobs 1 (0.32 s),
native job windows 19 (6.12 s), native operation windows 10 (2.74 s).
Two legacy tests originally assumed immediate close/cancel; updated fixtures
assert the question sends no cancellation, then explicitly confirm while retaining
the exact-target/destination assertions. Final reruns passed. Initial failure logs
remain available. The earlier F7 fixture failure omitted the normal visible-pane
acknowledgement; production F7 behavior was unchanged.

Machine-readable evidence: `windows-engine-summary.json`, `windows-ui-summary.json`,
`linux-engine-summary.json`, `linux-ui-final-summary.json`, `linux-final-summary.json`.
Strict lint logs: `windows-clippy.log`, `linux-final-clippy.log`. Two unchanged
vendored femtovg warnings remain (unused `FilePath` import and `font_scale` method);
there were no first-party strict-lint failures.

Independent reviews covered configuration admission/destruction, Rename capture
and prepared-native ownership, Cmd argv/cwd, and confirmation session/race handling.
Actual cancellation-question renders at 320 x 200 were inspected in standard dark
and touch/high-contrast themes on both platforms. Both actions fit; Continue has
focus and a neutral checkmark. Linux harness font fallback is not a qualification
of every installed desktop font configuration.

## Fresh Windows debug runtime

`cargo build` completed in 43.27 s. The executable is:

`D:/experiments/dev/FM/_build/debug/me.exe`

- Size: 99,159,552 bytes.
- SHA-256: `b3fdb18826381c704e5b3133e3dbde5e8fb11a3cf541d7435c1c279d35028a67`.
- Registered private runtime:
  `C:/Users/tjelinek/MidnightExplorer-20261112-confirm-rename-cmd-b3fdb188`.
- Matching unchanged plugins were deployed from the previous verified runtime;
  no ABI change required a plugin rebuild.

The fresh executable passed backend startup against Git/bin and GUI startup
against System32/repository with both native and forced-software renderers.
All three exited 0 with empty stderr; user preference content was unchanged.
`run-final-smoke.ps1` records the exact commands. Evidence is
`final-smoke-backend.log`, `final-smoke-native.log`, `final-smoke-software.log`.
Only the owned temporary deployment staging was removed after registration,
reclaiming approximately 0.532 GiB; the new executable/runtime remain available.

These are affected-suite and startup checks. They do not assert completion of all
backlog features, physical Linux desktop validation, or native GPU scrolling/frame
pacing qualification. CI status belongs to the subsequently pushed commit.
