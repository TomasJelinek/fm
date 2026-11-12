# Inline tree directory-size verification

This slice connects the existing directory-size workflow to adopted native and
archive trees. It does not complete other tree action consumers or the broad
application goal. The [plan](../superpowers/plans/2026-11-10-tree-folder-sizes.md)
and [runtime contract](../developer/TREE_PANEL.md#directory-sizes) describe the
scope. Generated logs live under `_build/qa/tree-folder-sizes/`.

The later [expanded-folder scope correction](2026-11-10-expanded-folder-sizes.md)
supersedes the top-level-only unselected behavior recorded below. These original
results retain their historical scope.

## Observable behavior

- Ctrl+Shift+S measures marked ordinary directories, including explicitly marked
  descendants; no marks means selectable top-level directories. Selected files
  alone never cause fallback traversal.
- Space measures the original focused directory independently of other marks,
  toggles selection and advances the caret. Insert only toggles and advances.
  Deferred full requests cannot silently retarget a newer selection; accepted
  jobs continue across later selection changes.
- Native and archive values use stable occurrence IDs. Progress does not change
  the row projection or order. Explicit Size sort uses known calculated values;
  filters and selection changes preserve them. Archive scans inspect metadata,
  not extracted content, including overlapping ancestor/descendant selections.
- Row formatting and status use the existing locale-aware byte totals and
  partial/running styles. The error glyph denotes an actual error, not ordinary
  unfinished work. Exact selection receipts fence selected-folder summaries.

## Regression and review evidence

The initial UI RED proved the bulk action was blocked and Space emitted only
the selection toggle. Backend capture RED proved missing top-level admission and
exact selection-allocation validation. A separate UI regression proved that a
deferred bulk request must not measure later marks.

Independent review identified two implementation defects before delivery:

- An ordinary eight-command mailbox could reject a size acknowledgement after
  outer-channel admission. A dedicated bounded acknowledgement slot now bypasses
  that queue; the saturation test fills eight slots, rejects a ninth ordinary
  command, accepts the acknowledgement and still drains all eight commands.
- A rejected stale Refresh invalidated accepted size work. Validation now precedes
  invalidation. A controller regression measures real fixture directories,
  explicitly sorts them by size, rejects an obsolete Refresh and forces another
  projection while asserting retained values and order.

An existing controller regression also caught unsolicited size-summary events
before any measurement. Summary publication is now enabled only after a size
request and is reset on navigation. Its failing and passing logs are retained.

The State lifecycle tests exercise blocked-call ownership after cancel/navigation,
final publication after acknowledgement/time without new input, overlapping
queued running marks, the 250 ms map cadence, exact selection receipts and
refresh version/acknowledgement continuity. Archive-specific tests revoke the
source after capture and require an error instead of successful zero; Single
uses the captured raw child even while its ancestor is marked. Their native-I/O
fixture panics if archive scanning falls back to native paths.

## Windows qualification

Before the final shared capture-helper refactor:

- 571 Engine unit tests passed, one existing test ignored; 48 core units passed.
- The 92-test tree subset, 15 controller integrations and 11 filter integrations
  passed. These are overlapping subsets, not additional unique unit totals.
- 702 UI units and 21 integration tests passed: seven new size bridges, four
  filter, five prefix, one tree panel, three disclosure and one size-error test.
  `ui/final-green.log` records the combined 723 passing cases.

The new bridge uses the actual Engine, Slint controls and presenter with controlled
providers. It verifies measured lengths 1,000/12/0 bytes, top-level total 1,012,
overlapping archive totals, status values, three layouts, Space's original target,
files-only selection and explicit Size sort followed by filter/clear.

Its first filter test incorrectly expected a name mask to hide folders. Masks and
Regex intentionally keep folders; the corrected test selects Expression through
the actual dialog and uses `name = "small"`. No production filtering policy was
changed to satisfy that expectation.

## Linux qualification environment

The first two broad Linux runs correctly rejected Make File List publication on
the Windows-mounted checkout filesystem (DrvFs). The fixture had hardcoded
`CARGO_MANIFEST_DIR/../../_build/tmp`, so changing TMPDIR alone did not move it.
It now follows the existing native transfer-test convention:
`std::env::temp_dir()/midnight-file-list/_build/tmp`. With the runner's native Linux
TMPDIR, 48 core and 562 Engine unit tests pass, one existing Engine test ignored.
Original failure logs are preserved; no production publication checks were relaxed.

Linux also passed all 697 UI unit tests and 18 bridge/layout regressions, including
the seven new size bridges. After extracting the shared capture helper, Linux
requalified 92 tree units and all 15 controller plus 11 filter integrations.
Logs are `linux/ui-units.log`, `linux/ui-integration.log`,
`linux/final-helper-tree.log` and `linux/final-helper-controller.log`.

## Final source and deployment

The final refactor removes duplicated target admission from generic selection and
directory-size capture. Both use the same count/byte-budget checks, typed target
resolution, context revalidation and cancellation checks. The size path uses a
borrowed iterator instead of allocating an intermediate row vector. Windows
requalified 92 tree units, all 571 Engine units and the 21 UI integrations against
this final helper. Independent source review approved both the original fixes and
the final helper/summary gate.

Core/UI all-target Clippy passed with `--no-deps -- -D warnings`. Engine's strict
run retains five existing dead-code groups for unfinished action consumers;
its all-target run with only that lint excluded passed. This does not claim a
warning-free full workspace. Focused formatting and whitespace checks passed;
the source-size helper checked 1,617 handwritten files, all at most 100,000 bytes.

The final Windows debug executable is `_build/debug/me.exe`, SHA-256
`1282969b6bd82a869212c417fa09d2afbd0cfe2b82e3771b0716b1ed08a65f33`.
Its identical protected copy is at
`C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe`; the build hash was
registered against the existing plugin runtime. The actual build-path executable
passed `--smoke-test`, listing five drives and both panels (30 and 132 entries).
`app-build.log`, `build-hash.json`, `deployment.log` and `app-smoke/` retain the
build/deployment/startup evidence. No plugin binaries or third-party dependencies
were changed by this slice.

## Limits

Native traversal retains the existing live path-based estimate contract, including
explicit link exclusions and partial/error accounting; it is not a filesystem
snapshot. Software-window tests do not establish physical desktop input, release
scroll performance, all native filesystem behavior or arbitrary plugin support.
Archive tests exercise captured native/archive adapters and metadata traversal,
not all deployed archive formats. Remaining tree mutations, clipboard, viewers,
checksums, drag/drop, containers and watch/refresh retention stay in the backlog.
