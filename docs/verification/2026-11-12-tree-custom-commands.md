# Native Custom Commands from inline trees — 2026-11-12

## Gap and implementation

The original checksum task was already integrated at commit `1c1b0b6`: typed
capture, common checksum execution, cancellation and actual Ctrl+F8 window tests
already existed. The remaining native Custom Commands path was guarded. This
change adds immutable typed tree sources to the existing command preparation and
launch pipeline rather than another process launcher.

Activation freezes configuration, native active/other locations, exact view and
selection revision, and focus independently of marks. Once uses that focus;
Per selected item and grouped argv preserve selected display order and fall back
to focus only when unmarked. A literal Once command permits no file target,
including the parent row. Provider targets supply native values on workers;
context-only rows cannot become arguments, mixed archive/native arguments fail,
and neither captions nor TEMP exports supply authority. The captured native root
must equal the original active location.

Native preparation reuses existing versioned entry/directory pins and preflight.
Provider owners remain alive through a blocked native launch and retire off the
caller. Initial preparation validates the whole capture. Per-process checks use
only immutable entry IDs in its existing pin scope; indexed target lookup avoids
an additional full-selection walk per process. A 256-item regression counts those
scopes without an arbitrary timing threshold. Unknown scoped IDs fail.

Pending selection acknowledgement, capture queue pressure and launch transport
refusal retain the exact intent. Provider budget failures are terminal, distinct
from transient admission pressure. Navigation, reprojection, settings-preview
edits and shutdown reject pending/late receipts. Accepted launches survive later
panel navigation; selection and expanded rows remain unchanged.

## Validation

The corrected Linux targeted no-run completed with the configured SDK and shared
`_build/linux` target. `scripts/run-targeted-tests.py` selected **61 cases across
seven artifacts**, all passed, none ignored. Runner wall time was **16.856 seconds**,
with peak four test processes and total libtest thread budget four:

| Artifact/filter | Passed | Runner elapsed |
| --- | ---: | ---: |
| Engine lib: typed capture, common command actor, archive retirement | 20 | 1.071 s |
| Native tree preparation integration | 10 | 4.404 s |
| Existing flat preparation integration | 8 | 1.436 s |
| Existing command actor integration | 2 | 0.537 s |
| UI presenter: tree and existing flat command handoff | 17 | 0.959 s |
| Window/native preparation with controlled launch service | 2 | 4.443 s |
| Window/actual native process and child cleanup | 2 | 10.748 s |

Engine filters matched `tree_user_command` (8), `user_commands::` (9) and
`archives::size_sort_tests::` (4); one actor case overlaps and runs once. The
archive filter verifies the independently corrected final-retirement receipt
regression. UI filters matched six new tree and eleven existing flat handoff cases.
Each preparation artifact includes three existing resolver cases through its
shared source fixture; counts above describe executed cases, not unique functions.
The 256-item scope-count test verifies both complete initial validation and one
immutable source identity per ordinary PerItem process, plus unmarked Once focus.

The window tests use installed Rust/Slint callbacks, the production Engine and
real native filesystem pins. Controlled-service tests verify left focused nested
folder Once and right marked nested-folder/root-file ordering. The additional
native-process cases are described below. This is a correctness run, not a
throughput or native foreground performance claim.

The full Linux gate evidence is under `_build/qa/tree-user-commands-20261112/`:
`build-corrected.jsonl`, `linux-corrected/summary.json`,
`linux-corrected-wall-ms.txt` and `clippy-corrected.log`. Targeted strict Clippy
(`-D warnings`) passed for core, engine, UI and their selected integration targets.
Earlier Clippy findings were a test's unused preview-conversion method, an
if-let-chain style correction, and a fixture default-initializer style correction;
all were fixed without production lint suppression. The shared probe module's
unused direct-child waiter is allowed only in the real-process test, where Engine
owns polling instead.

After the deterministic directory-revision fixture correction, the final Linux
follow-up passed **12/12 cases**: tree preparation10, Salamander shortcuts1 and
selected-item navigation1. Runner wall time was **7.936 seconds**, with peak three
processes and total libtest thread budget three. The final selected core/engine/UI
strict Clippy run also passed (59.40 seconds). Evidence: `build-revision.jsonl`,
`linux-revision/summary.json`, `linux-revision-wall-ms.txt` and
`clippy-revision.log`.

Owned Rust formatting, UTF-8/LF/source-size checks and `git diff --check` passed.
Independent integration review found no remaining blocking issue after replacing
repeated whole-selection revalidation with exact per-process identity scopes.

## Windows findings and actual processes

The Windows integration initially refused a valid focused directory before launch.
Diagnostic evidence recorded identical directory kind, attributes16 and absent size,
but cached enumeration modified time `134389566721460750` versus fresh handle time
`134389566721470818` (FILETIME units, approximately 1.0068ms). The common preparer now
ignores directory display size/mtime before establishing its pin. Kind checks,
regular-file metadata checks and every post-pin native identity/revision check
remain. Deterministic regressions cover stale directory display metadata, actual
post-pin directory timestamp change/replacement, and stale regular-file refusal.
The first Windows revision fixture relied on child insertion immediately advancing
the parent directory timestamp, which Windows did not guarantee. The final fixture
uses the native metadata service to set and read back an exact directory timestamp
after pinning, then requires revalidation to fail; replacement is tested separately.
No production post-pin check was weakened to accommodate that fixture.
Provider path leases do not promise filesystem identity at the earlier row capture.

Two additional integrations invoke the existing owned argv/cwd probe through the
real `LocalFileSystem` command service. They use Unicode nested directory/file
arguments, verify the exact job summary, wait for successful actual child exit,
and observe native handle Drop off the UI caller before reading reports/cleanup.
The test adapter only observes the real service and returned process; it introduces
no launcher or production code. Linux initially passed both native cases plus the
six presenter and two controlled-service cases (`linux-process/summary.json`).

Windows also exposed two fixture assumptions: `/` appended to a verbatim Windows
path is not a valid native component, so templates now use `MAIN_SEPARATOR`;
`GetCurrentDirectory` reports the ordinary DOS spelling after a verbatim cwd input.
The Windows fixture verifies both live directory identities before allowing only
that prefix difference, then compares every remaining UTF-16 byte. Argument bytes
remain exact.

The final Windows runner passed **63/63 cases across nine artifacts**, none
ignored, in **20.978 seconds**, with peak four processes and total libtest thread
budget four:

| Artifact/filter | Passed |
| --- | ---: |
| Engine typed capture, command actor and archive retirement filters | 20 |
| Native tree preparation | 10 |
| Existing flat preparation | 8 |
| Existing command actor integration | 2 |
| Tree and flat presenter handoff filters | 17 |
| Controlled-service tree window integrations | 2 |
| Actual native-process tree window integrations | 2 |
| Salamander shortcut and selected-item navigation integrations | 2 |

The native-process cases verify focused nested Once and marked PerItem arguments,
raw Unicode argv/cwd, exact job completion, successful child exit and native handle
retirement off the UI thread. The final successful no-run receipt is
`windows-revision-build.jsonl`; execution evidence is
`windows-revision/summary.json` and `windows-revision-wall-ms.txt` under the same
QA directory. These Windows tests and the Linux runs qualify native launch and
cleanup through the existing service; they do not qualify desktop key injection.

## Early-rejection ownership follow-up

Final independent review found that `Controller::start` receives a raw request
before ID/configuration/service/budget validation. The UI retirement clone could
already have drained, leaving early rejection to destroy the final captured
provider on the dispatcher. Two focused regressions reproduced this on Windows:
actual captured provider Drop matched the caller for invalid admission and for
final release without a Tokio runtime (`capture-retirement-red.log`, 0/2 passed).

The capture producer now reserves its existing tree retirement lease before
acquiring selected/focused/root provider targets. It publishes a `WorkerOwned`
wrapper around the entire source payload, including projection, selection,
registrations and the capture permit. The existing shared four-capture semaphore
bounds rename, metadata and command ownership; no additional queue, worker or
unbounded fallback was introduced. The permit is returned only after payload
retirement. Failed construction stays on the capture worker, and reservation
refusal occurs before provider acquisition.

The regression removes the original session and all UI clones, then covers
invalid, unavailable, stopped, saturated cleanup and full admission queue paths.
A separate case proves retirement works without Tokio. Existing capture tests now
await actual admission-permit return instead of assuming synchronous Drop; an
additional case checks cleanup saturation performs no provider capture.
Final targeted follow-up passed **19/19 on Windows and 19/19 on Linux**, none
ignored. Grouped filters matched `tree_user_command`11 and `user_commands::`11;
three overlapping cases ran once. One artifact/process with one libtest thread
ran per platform; runner elapsed times were 8.519 seconds and 1.082 seconds,
respectively. Strict Linux engine Clippy (`--lib --tests -- -D warnings`) passed
in 1m58s. Evidence under the same QA directory: `capture-retirement-red.log`,
`capture-retirement-{windows,linux}-build.jsonl`,
`capture-retirement-{windows,linux}/summary.json`, and
`capture-retirement-linux-clippy.log`. The seven changed Rust files passed
rustfmt100, UTF-8/LF, 100KB and diff-whitespace checks. Independent review approved
the complete payload ownership and shared capacity accounting; no finding remains.

## Limits

The final Windows host was rebuilt and deployed with the corrected capture
ownership. Four controlled-service/native-process integration cases pass against
that final build; [debug integration evidence](2026-11-12-tree-images-debug.md)
records startup, hashes and scope. The owned probe does not qualify arbitrary
configured programs, terminal behavior, external process effects or native
desktop input.

Archive/provider command exports, directory comparison, remaining context menus
and general TREE-01 completion are not claimed. No plugin ABI or dependency is
changed by this feature.
