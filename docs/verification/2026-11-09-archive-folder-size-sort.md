# Manual archive Size sorting

Date: 2026-11-09. This follows the metadata scanner slice documented in [archive folder sizes](2026-11-09-archive-folder-sizes-backend.md) and its [presenter integration](2026-11-09-archive-folder-sizes-ui.md). Backend qualification is complete on Windows and native Linux. This report does not claim an installed application workflow test.

An explicit archive Size sort now uses finished, error-free calculated directory totals from the backend's last published immutable snapshot. Partial, unfinished and missing directory totals remain unknown and sort last in either direction. Links are not measured. File lengths, folders-first grouping and deterministic name/ID ties retain existing rules. Raw Entry metadata and ArchiveNode sizes remain unchanged.

The existing ArchiveNavigate command already carries SortSpec. Its listing worker captures a size snapshot only when the weak recorded mount is the exact same Arc allocation as the live target; an equal numeric mount ID is insufficient. Source validation brackets listing and sorting. The immutable lookup is passed to the shared core sort_entries_with_sizes comparator. Progress publication does not invoke navigation or sorting, and there is no new UI command or UI-supplied map authority.

The size worker records weak source and snapshot references atomically with successful event enqueue under a short backend state lock, before wakeup. The lock is released before event backpressure waits. A concurrent receiver's explicit sort cannot race ahead of recording the map. Failed publication preserves the previous record; invalidation removes only the matching source. Weak references retain neither idle archive permits nor a retired map's allocations. Existing exact pane/mount/version ACK bounds remain intact.

Evidence lives under _build/archive-folder-size-sort. The initial Linux regression failed with IDs 2,3,4,5,6 rather than calculated order 6,3,2,4,5; a source-change regression also failed before validation was added. Six new engine tests plus the strengthened existing enqueue/cancellation regression cover asc/desc, empty/partial/unknown/link totals, duplicate names, source replacement/change, raw metadata, weak retirement, publication failure/invalidation and wakeup visibility. All content-read fixtures panic if extraction is attempted, with explicit zero-read assertions in navigation tests.

Independent source review found no actionable issue in the manual-sort implementation and inspected the RED/GREEN logs without rerunning tests.

| Check | Windows | Native Linux (WSL Ubuntu 24.04) |
| --- | ---: | ---: |
| Entire core test suite | 136 passed, 1 subprocess fixture ignored | 134 passed |
| Engine library | 506 passed, 1 manual ignored | 497 passed, 1 manual ignored |
| Native folder-size integration | 20 passed | 20 passed |
| Selected-folder status integration | 5 passed | 5 passed |
| Archive-selection integration | 2 passed | 2 passed |
| Volume integration | 5 passed | 5 passed |
| Core all-target strict Clippy | Passed | Passed |
| Engine all-target qualified Clippy | Passed | Passed |

The engine lint gate allows only dead_code and unused_imports for the existing 13 TREE-01 baseline diagnostics; this is not an unmodified strict-engine pass. Formatting and the 100,000-byte source-size limit pass for this slice's files. Linux additionally passed the parallel locale slice's navigation_ordering (5) and desktop (9) integrations. No dependency, SDK, archive worker package, native ABI or UI source changed.

Evidence: green-windows.log contains the focused 4 navigation + 23 archive-size checks and full core suite; final-windows.log and final-linux.log contain broad final gates. red-linux.log contains both intended initial failures. windows-source-hashes.json and source-hashes.json record qualification snapshots; shared lib.rs also contains the parallel locale task's setup change. changed-scope.txt identifies only this slice's edits, since whole-file hashes are not a repository-wide changed-file inventory. Verification scripts record exact environment and commands, using existing dedicated archive-folder-sizes/windows and /linux targets.

The persistent archive-size blocking lanes exposed an existing volume-cache test assumption: pinned Tokio 1.54.1 disables paused-clock auto-advance while blocking tasks run. The test now explicitly advances to its existing 45-second deadline plus one millisecond, preserving the upper bound and refresh assertions. The observed hang is retained in _build/qa/navigation-ordering/volume-clock-stages.log; all five volume tests pass on both platforms after the correction (volumes-windows.log and final-linux.log). Production worker behavior was unchanged.

Expanded-tree sizing remains a separate authority integration. Session::capture_selection (tree_action_capture.rs) already admits the exact acknowledged projection/selection and retains bounded CapturedTarget registrations and contexts. A future tree-worker command must carry that view, selection revision and optional explicit Space row, then pass backend-owned captured archive directory authority to the scanner. Captions and copied ArchiveLocation payloads cannot supply it. Ancestor/descendant overlap and display TreeRowId correlation require explicit semantics.

A critical cache boundary is providers/target.rs: each resolved target gets a new LeasedArchive wrapper Arc retaining TargetContext. Future tree integration must preserve that context while introducing canonical backend source identity, or isolate caches per wrapper. Matching only numeric mount IDs would reintroduce replacement-source reuse. Unlisted root targets, links, mixed native/archive selections and nested archives need explicit admission behavior. The detailed research proposal is _build/archive-folder-size-sort/expanded-tree-capture.md; no expanded-tree implementation is included here.

Prepared runtime acceptance lives under _build/archive-folder-size-sort/ui-acceptance: an owned ZIP, manifest, verified entry lengths, regeneration script and acceptance.md. The fixture has folders of 120, 3 and 0 bytes and one 7-byte root file. The steps use --ui-smoke-test in-memory preferences, record before/after preference and archive hashes, and require the combined candidate built after this sort slice. Preparation did not launch an application or claim an installed workflow result.
