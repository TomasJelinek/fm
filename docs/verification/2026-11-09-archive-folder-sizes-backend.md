# Archive folder size backend verification

Date: 9 November 2026. This covers the metadata scanner, bounded controller,
captured-source authority, result cache and internal command/event contract.
[Presenter and software UI checks](2026-11-09-archive-folder-sizes-ui.md) are separate;
these tests do not claim an installed application hotkey workflow.

## Behavior and authority

Ctrl+Shift+S computations and single-folder Space requests use the exact completed
TreeRootSnapshot captured through SnapshotState::capture_tree_root. The retained
ReadyRoot owns the existing archive mount. Arbitrary UI ArchiveLocation values,
equal-content replacement allocations and synthesized native member paths are
not accepted as authority.

The iterative scanner lists metadata and sums stored logical file lengths. It
never calls ArchiveMount::read. No selection means all immediate ordinary folders;
an explicit empty or files-only selection scans none. Every visited subdirectory
receives its own result, keyed by VirtualEntryId, so duplicate names and sorting
cannot retarget values. Links are excluded. Unknown lengths, unreadable branches,
malformed identities and checked-add overflow produce explicitly partial totals.
Captured ancestors and all immediate sibling IDs are reserved before traversal;
an unselected sibling cannot be aliased into a selected subtree.

Two persistent workers own scan work and final reclamation. Each pane has at most
eight queued requests, one coalesced observer and two mount caches, each bounded
to the existing 100,000-node metadata budget. Pending traversal retains directory
IDs rather than full sibling metadata at every depth. Active/queued scans retain
source authority; idle observers, caches and deferred publications use weak
authority so leaving an archive releases its worker permit.

Scans publish progress at most every 250 ms plus final completion. They do not
sort, cancel on ordinary interaction, or restart because the same mount is sorted.
Live source validation surrounds reads, cache reuse and publication. Invalidated
events name the exact mount whose displayed sizes must be retired.

Each pane permits one immutable snapshot in flight. Acknowledgement requires its
exact pane/mount/version, including snapshots the UI discards. Until acknowledgement
the worker keeps a small weak descriptor and updates its mutable cache, avoiding
event-queue multiplication of large maps. Finished can precede a deferred final
snapshot; consumers accept newer versions for their current mount independently
of whether that scan is still marked active. Successful enqueue retains its ACK
token even if the wakeup cancels the scan. Selected-folder totals are computed on
the backend; the UI performs only individual identity lookups and retires maps
through its backend reclaimer.

## Results

Dedicated targets: `_build/archive-folder-sizes/windows` and
`_build/archive-folder-sizes/linux` (native WSL Ubuntu 24.04).

| Check | Windows | Native Linux |
| --- | ---: | ---: |
| Core library tests | 30 passed | 30 passed |
| Final engine library tests | 500 passed, 1 manual ignored | 491 passed, 1 manual ignored |
| Existing native folder-size integration | 20 passed | 20 passed |
| Existing selected-folder status integration | 5 passed | 5 passed |
| Existing archive-selection integration | 2 passed | 2 passed |
| New archive-size tests, included above | 21 passed | 21 passed |
| Core strict Clippy | Passed | Passed |
| Engine strict Clippy | Existing TREE-01 warnings | Existing TREE-01 warnings |

Formatting passed. Unmodified engine `-D warnings` reports 13 pre-existing unused
import/dead-code diagnostics in the unfinished tree action-capture implementation.
Those files were not changed by this slice. A separate all-target Clippy invocation
allows only `dead_code` and `unused_imports`, keeping every other warning denied.
Both platforms passed that invocation. This qualified lint result must not be described as a clean
unmodified strict-engine gate.

The 21 focused tests cover nested and empty folders, duplicate names, selected
scope, Space/cache reuse, links, malformed IDs/parents/mounts, captured sibling and
ancestor aliases, 4,097-directory iterative depth, stale sources/captures, overflow,
unknown sizes, bounded queues, supersession, explicit cancellation, monotonic
snapshot acknowledgement, observer retirement and sorting during an active scan.
Every fake mount rejects content reads; relevant tests also assert a zero read
counter. The initial five scanner tests were observed failing before implementation.
The mid-scan same-mount sort regression was observed timing out before its fix.

Independent review found and corrected: idle observer mount retention; missing
captured-sibling reservation; snapshot queue multiplication; same-mount newer-view
suppression of an older scan's final result; and successful enqueue being mistaken
for failure when its wakeup cancels. Deterministic regressions cover these boundaries.
The final independent source review reported no remaining actionable findings;
the reviewer inspected source and logs without rerunning the builds.

Evidence under `_build/archive-folder-sizes/`:

- `windows-verification.log`, `linux-verification.log`: broad core/engine and
  existing integration gates, plus the original strict-Clippy outcome.
- `final-windows.log`, `final-linux.log`: final engine tests and qualified lint
  after publication-race review fixes.
- `review-tests.log`: all 21 focused tests on Windows.
- `red.log`, `sort-red.log`: observed failing scanner/sort regressions.
- `windows-clippy-strict.log`, `linux-clippy-strict.log`: exact baseline diagnostics.
- `source-hashes.json`: frozen owned source hashes; not a changed-file inventory.
- `verify-*` and `final-*` scripts: exact commands/environment setup.

No third-party dependency or exported archive-worker package changed in this slice.
The prior streaming package evidence remains separate.

## Remaining integration boundaries

Expanded-tree descendant selections need their own captured tree action authority;
the flat-root scanner explicitly rejects those IDs. The subsequent
[manual Size-sort slice](2026-11-09-archive-folder-size-sort.md) connects calculated
totals to explicit archive sorting while preserving the rule that progress never
triggers automatic sorting. Its qualification is recorded separately.
Installed application acceptance is owned by the parent integration task.

Source checks are live validation, not an immutable filesystem snapshot. A provider
call that blocks inside the OS keeps its bounded worker/authority until it returns.
Small control events retain existing event-channel backpressure; normal application
shutdown closes that receiver before dispatching Shutdown. Salamander metadata-size
references and the no-copied-GPL-code boundary are recorded in the
[design](../superpowers/specs/2026-11-09-archive-folder-sizes-design.md).
