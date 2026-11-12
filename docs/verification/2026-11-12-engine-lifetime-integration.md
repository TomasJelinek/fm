# Engine lifetime integration — 12 November 2026

This batch follows `6f06ec4`. Copy/Move/Delete continue to share the same progress
window and cancellation policy. The changes here protect backend ownership during
cancellation and shutdown; they do not change operation authority or verification.

## Implementation and remaining scope

Startup reserves one plain cleanup worker, 74 payload slots and three waiter tokens
before constructing providers. An unstarted Engine and the complete pinned run future
are guarded before first poll. The command receiver closes and drains even outstanding
pre-close send permits on that worker. Shared panel snapshots and drag state protect
actual last owners; Copy and mutation guards retain preparation, native/provider
owners and admission resources through awaited states and actual destruction.
See [the engine contract](../developer/ENGINE_LIFETIME.md),
[primitive qualification](2026-11-12-retirement-primitive.md) and
[actor qualification](2026-11-12-actor-retirement.md).

This is partial LIFE-01 remediation. The independent review identified remaining
service-clone and request ownership in Find, checksum and Custom Command actors,
and repeated builder replacement. Those findings remain open in
[the review](2026-11-12-engine-lifetime-review.md). No blanket thread-safety or full
shutdown coverage claim is made. A blocked native destructor or indefinitely held
command permit can stall this bounded worker; caller/UI Drop never joins it.

## Windows evidence

Generated logs are under `_build/qa/life01-engine-20261112/`.
`red.log` reproduced both original unpolled Engine/queued-capture caller destruction
failures. Actor and primitive RED evidence is linked above. The final root tests cover:

- no-runtime unstarted Engine and unpolled run destruction;
- one consumed capture blocked in output delivery plus a second queued capture;
- runtime shutdown before first poll and after a deliberately pending first poll;
- normal Shutdown acknowledgement with queued captures;
- a command delivered through a permit reserved before receiver close;
- interruption during tree shutdown while Tokio blocking capacity is deliberately occupied.

The additional snapshot and unattached-drag tests cover their final owner after runtime
loss. Tests use observed state, explicit gates, acknowledgements and bounded watchdogs.
The runtime-after-first-poll fixture initially used synchronous Runtime Drop while live
tree workers still belonged to the retained run; that fixture deadlocked. It now uses
the application's bounded shutdown model before dropping the run. The initial combined
log preserves the failed attempt. The archive size-sort ownership fixture also now
awaits the exact retirement acknowledgement before checking that its weak source expired;
it no longer assumes asynchronous cleanup is synchronous.

`affected-units-green.log` passed these complete affected Windows unit suites:

| Package | Passed | Existing ignored | Test-body time |
| --- | ---: | ---: | ---: |
| core | 68 | 0 | 0.09 s |
| elevation | 7 | 0 | 0.05 s |
| engine | 893 | 7 | 6.80 s |
| platform-windows | 174 | 12 | 5.40 s |
| ui | 903 | 4 | 3.43 s |
| ui-input | 29 | 0 | <0.01 s |
| viewer-engine | 187 | 4 | 0.38 s |
| viewer-render | 9 | 0 | 0.13 s |
| viewer-ui | 115 | 0 | 1.00 s |
| **Total** | **2385** | **27** | |

These are affected application unit suites, not all independent plugin workspaces.
The subsequent compact PDF footer correction has its own targeted UI qualification.
Final strict Windows Clippy passed for core, engine, application, UI and viewer crates,
including all targets (`clippy-windows.log`). The existing vendored femtovg warnings are
outside the `--no-deps` project gate. Final named-service grouping passed the 209 operation
tests again in 5.98 s (`operations-final-windows.log`).

## Linux targeted evidence

The independent `_build/linux` target uses the pinned Linux SDK configuration and a
unique private test fixture. Final targeted results: lifecycle 8/8 in 0.07 s,
retirement-filter cases 37/37 in 0.36 s, operation-filter cases 200/200 in 7.21 s,
archive size-sort cases 4/4 in 0.12 s. Strict core/engine all-target Clippy passed.
The operation count excludes Windows-only cases; it is not reduced test coverage by filter.
Full Linux workspace and hosted CI results remain separate gates.

## Final native operation and navigation integration

The Windows engine integrations passed 48 tests, with two existing optional cases
ignored (`engine-integrations-windows.log`): navigation ordering 5, real filesystem
copy/move operations 14, tree controller 18, and tree filters 11. The real operation
fixtures compare independent SHA-256 manifests, validate same-volume identity-preserving
rename, self/hardlink rejection, directory merges, cancellation and retained originals.
These operate only on test-owned directories; no user documents are modified.

The corresponding UI boundary passed 24 tests (job progress 2, native job windows 14,
real native operation UI 4, tree filters 4; `ui-integrations-windows.log`). The separate
shared-operation suite passed 10/10 (`shared-progress-windows.log`), including Trash
and permanent Delete using the same modeless progress component as Copy/Move, unknown
totals, exact operation Pause/Cancel routing, and cancellation admission failure. The
640x420 Delete discovery render was inspected in `ui-renders/delete-progress-640x420.bmp`.
The Jobs ledger remains independent; cancelling a confirmation never inserts a job.
The corrected native performance probe passed all six focused tests before rebuilding.

## Fresh debug application

`debug-build.log` records the successful application/helper build. The final
`_build/debug/me.exe` has SHA-256
`dfbafca6af61e1ee23f7341131958110ed048ed384d2e7c9858419fce0c901cf`.
Its verified private runtime is
`C:/Users/tjelinek/MidnightExplorer-20261112-lifetime-dfbafca6`; hash-bound registration
lets the checkout executable use the same existing viewer/filesystem/thumbnail plugins.
No plugin binaries or public plugin APIs changed in this batch.

Backend, native-renderer and explicit software-renderer smoke cases each exited zero
with empty stderr. Both panel screenshots were inspected; application preferences
were unchanged. Evidence: `final-smoke-*.log`, `final-native.png`, `final-software.png`.
The verified deployment staging copy was removed afterward, recovering 0.531 GiB.
Format/diff checks passed and all 2024 checked handwritten files remain at most 100000 bytes.

The separate scrolling qualification reports remaining native GPU timing failures;
this successful application startup is not a scrolling-performance claim.