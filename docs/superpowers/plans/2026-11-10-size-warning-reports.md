# Directory-size warning reports implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development or
> superpowers:executing-plans, with failing regressions before implementation.

**Goal:** Every red/yellow warning or exclamation icon in file panels has a
specific diagnostic tooltip and opens full details when clicked. Yellow size
warnings show actual unreadable paths and reasons for native, archive and
inline-tree scans, without stopping useful work or retaining an unbounded
in-memory error list. Red listing/application errors remain distinct.

**Architecture:** Attach a cheap retained report handle to each affected size
result. `FolderSize` becomes Clone rather than Copy; the handle follows the
existing cached result and expires with its last owner. Write diagnostic records
to existing private temporary storage, freeze complete-record prefixes on demand,
and open them through `GeneratedTextSource` and the basic viewer. Archive DFS
subtree byte ranges share records instead of duplicating child diagnostics.

**Tech stack:** Existing Rust/Tokio services, Slint, native private spool and
generated-text viewer. No new dependencies or provider ABI changes.

**Spec:** User request for yellow inaccessible-folder warnings and a clickable
list of paths and reasons; `_build/qa/size-warnings/design-inspection.md` records
the source inspection. Its earlier numeric-ID/paging proposal is superseded by
the retained report contract agreed with the coordinator.

## Constraints and review focus

- All filesystem work and final report destruction run on backend workers.
- No new source/path byte ceiling, no per-error UI events and no unbounded
  error-count-sized vector. Scanner logs stream records; tree refresh composition
  may retain source owners bounded by the existing tree-node cap, without copied
  path/reason strings. Traversal, concurrency and four-update-per-second budgets stay.
- Report storage failure must not stop counting. Show observed/stored counts and
  the storage failure; never describe missing diagnostics as a complete report.
- Native path/control characters and archive raw names must remain distinguishable
  in readable output. Display text never becomes operation authority.
- Cancellation alone creates no warning; real errors observed before cancellation
  remain visible. Completed and cancelled partial reports retain their own data.
- Snapshots must remain unchanged after further writes, reruns, navigation and
  cache eviction; readers cannot observe partially written diagnostic records.
- Windows shared file positions must not let concurrent reads overwrite appended
  text. Existing sealed archive/file-list spool behavior must remain unchanged.

## Task 1: append-only private spool and report source

Files: new `crates/midnight-worker-platform/src/spool_report.rs` and its tests;
small exports/factory in `spool.rs`; new engine `size_warnings.rs` and tests.

- [x] Add failing frozen-prefix, concurrent reader/writer, late storage failure,
  private lifetime and offset-bound tests.
- [x] Add `AppendSpool` and `SpoolPrefix`, reusing native create/read routines.
  Serialize file access with a shared mutex and explicitly seek before appending.
  A prefix can retain successfully written bytes after a later append failure.
- [x] Add streaming complete-record report generation and immutable generated-text
  snapshots with observed/stored counters and an honest failure footer.
- [x] Verify Unicode/long paths, errors larger than one I/O chunk, child ranges,
  cancellation, temporary-file cleanup and no path reopening.

Windows storage evidence: `_build/qa/size-warnings/spool-red.log` contains the
expected unsupported-prefix failures; `spool-green.log` passes all nine
worker-platform library tests. Linux `report-red-linux.log` records ten expected
stub failures, followed by ten passing report tests in `report-green-linux.log`.
`spool-green-linux.log` passes 21 worker-platform tests with two unrelated native
cgroup environment tests ignored. The subsequent Windows full-engine gate in
`_build/qa/settings-polish/windows-libs-green.log` also passed all ten report
tests. The report module is now connected to production scanners. The integrated
Linux gate `full-libs-green-linux.log` passes core 51 and engine 678 tests (seven
existing opt-ins ignored); `full-spool-green-linux.log` passes 21 worker tests
(two unrelated cgroup tests ignored). UI/deployment qualification remains separate.

## Task 2: connect every scanner and cache

Files: core folder-size/report/protocol types; engine folder_sizes/batched,
archive_sizes/scan and tree_sizes; existing result caches and publication paths.

- [x] Add exact native enumeration/metadata, missing length, invalid entry,
  capacity and arithmetic-overflow diagnostics without discarding FsError details.
- [x] Use archive DFS ranges for ancestor/descendant reports; validate captured
  archive path components and preserve existing revision checks.
- [x] Preserve reports through normal snapshots, cancelled partial totals,
  sorting/filtering, cache retention and backend reclamation.
- [x] Add the bounded report-snapshot request/result route and regression tests.

Linux `engine-routing-green-linux.log` passes two real Engine boundary tests for
full and individual native scans: acknowledge cancellation while enumeration is
blocked, retain the late completed-record prefix, then open/read that report.

Integration points:

- `folder_sizes::scan_root` records enumeration/metadata failures and explicit
  invalid-child, missing-length, frontier and overflow diagnostics. Native logs
  live per worker lane (four lazy spools), with per-root ranges;
  `batched::send_progress` still drops optional running
  totals under backpressure, while the report remains retained independently.
- `archive_sizes::scan::Frame` retains raw child directory names already charged
  to the existing name budget, plus a report range. One depth-first log per scan
  lets each completed child freeze its exact contiguous record range before
  later siblings run. The captured `ArchiveAddress` and raw traversal components
  identify paths; no display string becomes a native path or filesystem action.
- `tree_sizes::scan` reuses these scanners. Its `partial` path must retain the
  actual provider error and captured occurrence identity instead of discarding
  the reason. Cancellation without a real error does not create a report issue.
- `SharedSizes`, `entry_sizes`, `folder_size_totals`, archive `Cache.values` and
  `TreeSizeSnapshot` switch owned `FolderSize` copies to cheap clones/references.
  Cached results retain their own immutable report authority across re-sorts.
- UI native-map replacement and stale `FolderSizeUpdated` events must transfer
  old report owners to the existing backend reclaimer, just as archive/tree map
  snapshots already do. Small temporary clones remain safe while their map
  retains the owner; no final private-file close belongs on the UI thread.

## Task 3: clickable yellow UI and integrated qualification

Files: shared warning glyph/file rows, size presentation, presenter report route,
generated viewer launch, design/status/keyboard docs and verification evidence.

- [x] Every panel mode shows a yellow warning only for actual scan issues.
- [x] Every red/yellow file-panel diagnostic icon has a specific tooltip and
  a detail action, including failed tree disclosure and panel ErrorNotice.
  Clicking diagnostics must not silently retry or expand a folder.
- [x] Icon click captures the exact row/report; it does not select, activate,
  expand or drag the row. Keyboard and accessibility have equivalent behavior.
- [x] Open a stable searchable generated-text report; a missing basic viewer or
  report read failure produces an explicit readable error.
- [x] Test native/archive/tree workflows, reruns/navigation during pending open,
  cache eviction, live snapshots, storage failure and unchanged 250-ms publication.
- [x] Verify Windows/Linux and rebuild/deploy the Windows application, keeping
  qualification limits and broader unfinished work explicit.

## Tree listing and terminal diagnostics

Keep `TreeDisplayRow.error` bounded for row presentation. Retain a separate
immutable `GeneratedTextSource` for the original failure before the provider
tree's 4096-byte summary limit; no UI construction, source reads or encoding.
One lazy private report log per tree generation shares one backing file across
failed nodes. Node/checkpoint/projection copies retain only cheap source owners.
Retry clears the current node's error/source; an already opened snapshot remains
unchanged. Old appended bytes remain until navigation/session retirement and
the last retained snapshot release. There is no artificial diagnostic length
ceiling. Storage failures explicitly state how many details could not be saved.

- [x] Cover long Unicode provider failures, immutable details after retry and
  tree destruction, honest storage failure and backend final destruction.
- [x] Preserve full dynamic terminal tree failures too: `TreeDiagnostic` retains
  a separate source beside its 4096-byte summary. Backend producers preserve
  that source through initial construction, rejection and refresh failures.
  Root owns exact scoped panel correlation and backend UI reclamation.
- [x] Aggregate refresh details include every failed branch. Explicit branch
  work may interleave refresh, so compose exact immutable branch sources rather
  than taking a broad shared-log range. This stores only source owners bounded
  by the retained tree's node count, with no copied error/path vector or extra
  per-error file. Aborted refresh retains earlier failures and its final error.
- [x] Qualify lossless native-provider failure formatting on Linux (raw path
  units, native error kind and original operation/reason). Two distinct invalid
  UTF-8 paths with equal lossy captions now remain distinct in diagnostics.
- [x] Qualify the Windows unpaired-UTF16 counterpart in the coordinator's gate.
- [x] Validate gestures against captured pane/row identity and immutable tree
  stamp. A later row position, changed navigation or stale publication must not
  retarget a diagnostic gesture. Generated report names grant no path authority.

Final Linux backend gate `final-engine-green-linux.log` passes **684 engine tests**,
with seven existing installed-catalog opt-ins ignored. It additionally qualifies
late archive revision failures, real failures racing cancellation, Interrupted-
only cancellation without warnings, and reused watch-failure diagnostic snapshots
so unchanged cached errors do not repeatedly dirty the watch. Native worker-lane
logs retain appended bytes until lane writer retirement/engine shutdown; tree
generation logs retire on navigation/last reader. No storage completeness claim
is made after an explicit observed/stored mismatch.

Salamander source comparison is read-only: `src/mainwnd5.cpp` directory-read
failure paths format the actual directory and `GetErrorText` in an exclamation
message box. This supports specific path/reason presentation, not its legacy
buffer truncation or silent size-count error skipping. It does not establish a
retained clickable-report implementation.

## Integration handoff after settings qualification

Freeze the shared core contract before parallel edits: `FolderSize.warnings`
retains an optional cheap `SizeWarningReport`; its opaque source snapshots into
`GeneratedTextSource` on a backend worker. `OpenSizeWarnings` and the correlated
`SizeWarningsReady` result carry cancellation and retained authority, never a
display path or global numeric report lookup. Bound snapshot workers to two and
bound pending requests or return an explicit busy error. Retire stale/rejected
owners on the backend.

Prefer borrowed archive/tree/pane size getters. Clone report ownership only when
retaining a result or opening the report, avoiding reference-count work for every
rendered row. Audit final destruction during replacement, cancellation and UI
shutdown as part of the Copy-to-Clone migration.

The scanner review found an additional cancellation defect in
`archive_sizes::worker::run_scan`: stopped scans marked unfinished results finished
and added errors, including when the user cancelled. The archive slice now retains
unfinished estimates and real earlier errors without inventing a cancellation
error; Linux regression is green. Native cancellation can occur before the next
250-ms update; the final partial report is now drained and retained after the
native call actually returns, including after cancellation acknowledgment.

After the shared types are stable, independent owners can cover native scanning
and report workers, archive/tree scanners and range retention, and UI report
actions/generated viewer launch. Qualification must cover readable siblings after
denial, cancellation with/without prior errors, nonoverlapping archive ranges,
provider failure, storage failure with continued counting, immutable reports
after rerun/navigation, missing basic viewer, and off-UI final destruction.
