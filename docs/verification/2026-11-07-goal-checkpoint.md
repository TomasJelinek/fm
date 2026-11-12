# Transfer and tools checkpoint — 7 November 2026

This checkpoint implements several accepted tasks from the
[completion ledger](../IMPLEMENTATION_BACKLOG.md). It does not close the overall
goal or turn the remaining provider/platform designs into supported features.

## Implemented scope

- Verified Copy now distinguishes recoverable Replace/Replace all from directory
  Merge/Merge all. Changed targets require renewed consent; displaced originals
  remain in recovery storage. SHA-256 source streaming and independent complete
  destination readback remain mandatory.
- F6 and Cut/Paste use a captured Move draft and the backend queue. The current
  Move capability accepts at most64 ordinary native files. All files must pass
  publication, durability and complete source/destination proof checks before
  originals are captured into recovery folders. It retains their data and does
  not reclaim source disk space. Directories, links and archive Move are refused.
  [Transfer contract](../developer/TRANSFERS.md).
- Alt+F7 opens bounded, modeless Find sessions with filename masks and streamed
  literal UTF-8 content search. Result navigation requires backend validation.
  [Find contract](../developer/FIND.md).
- Empty-space dragging selects rows in Details, Brief and Thumbnails, including
  archive rows with opaque identities. Preview is immediate; the backend commits
  revision-checked ranges. [Selection contract](../developer/SELECTION.md).
- Ctrl+F8 opens modeless checksum jobs, defaults to SHA-256, and optionally
  computes SHA-512, SHA-1 and MD5. Individual hashes/rows can be copied; manifest
  output uses verified staging and exclusive publication.
  [Checksum contract](../developer/CHECKSUMS.md).
- Custom Commands has an ordered settings editor, transient Preview, conditional
  toolbar, right-click settings routing and configurable shortcuts. Program mode
  passes separate native arguments; Terminal mode uses literal authored scripts
  and explicit environment data. A launch job reports dispatch, not successful
  completion of the external program. Dragging rows in settings changes their
  order, preserves shortcut identities and saves once on a valid changed drop.
  Explicit images are decoded by the backend;
  automatic executable artwork remains pending.
  [Command contract](../developer/USER_COMMANDS.md),
  [settings](../developer/USER_COMMAND_SETTINGS.md).

All new native bodies remain separated by platform. No filesystem operations,
process spawning, hashing or image decoding were added to UI callbacks. Native
path identities remain distinct from bounded presentation strings.

## Windows UI regression evidence

The complete UI run covered the library and62 integration targets. Its only two
failures were obsolete Move-disabled and pre-Cut menu-position expectations.
The corrected fixtures and every subsequently changed presentation boundary were
rebuilt and rerun. The final focused run passed437 library tests and12 integration
tests (checksum3, focus1, compact menus3, command settings3 and toolbar2).

Combining the complete run with these fresh replacements gives589 passed,
zero failed and one intentionally ignored performance benchmark across63 groups.
The counts do not include duplicate reruns. Logs:
`_build/goal/commands/windows-ui-final.log`,
`windows-ui-focused-final-2.log` and `windows-ui-final-counts.json`.
The engine's archive selection Cargo integration also passed both tests in
`windows-archive-selection-final.log`.

New causal regressions include the editor's never-started validation timer,
right-click events consumed before opening the toolbar context menu, and shortcut
updates failing to notify cached toolbar rows. Each failed before its correction
and passed afterwards. The tooltip regression preserves command identity across
binding changes, bounds Unicode folder display and omits script/argument values.
Settings captures cover54 theme/density/size combinations; checksum controls
cover108 layout/action combinations. Selected normal and320×200 captures were
visually reviewed.

These tests dispatch real Slint pointer/key events through software-window
adapters. They do not by themselves qualify native OS windows, GPU rendering,
system clipboard integration or external process execution. The reported native
first-character keyboard-search crash remains an open investigation: an
unreproduced report is not claimed fixed by these independent corrections.

## Remaining qualification

Final Windows/Linux strict Clippy, the corrected Linux UI run, release build,
protected deployment and fresh native application/viewer checks are recorded
below when observed. Native process/transfer/backend evidence is maintained in
the linked feature reports; it is separate from these presentation tests.

The corrected Linux UI baseline and focused replacements passed586 tests across
the same63 groups, with one intentionally ignored benchmark. Strict Clippy passed
for Windows UI/application all targets and Linux UI all targets. Windows core
shortcut integration passed14 tests, including Move/Cut migration. Subsequent
manifest-destination and settings drag-order changes require their own fresh
focused checks; these totals describe the preceding source boundary.

A freshly compiled native Windows viewer probe passed all thirteen DX12 stages
in7.02 seconds, including text/binary/source/Markdown/image/PDF, controls, placement
and Escape. Source/PDF captures were inspected; this is actual Windows renderer
evidence, distinct from the software UI tests.
[Viewer evidence](2026-11-07-viewer-runtime-acceptance.md).

QA uses in-memory settings. The real preferences hash remains
`EE8B6D629D9931443C1BD39A9B8BD65C791E54D5F11EF86C7640E8C04E1DC6CB`.
Unhooked expression-filter and modified-time prototypes belong to the next
increments and are excluded from this checkpoint.

## Final command-ordering qualification

The final Windows affected-target run passed 479 tests; the final Linux affected
scopes passed 476. Both include all 12 actual settings widget tests. The portable
footer fixture was freshly rebuilt after accounting for different font metrics.
These are scoped final checks, not a claim that every earlier integration target
was rerun on the final source. [Settings evidence](2026-11-07-custom-command-settings.md).

The drag review also found and corrected ancestor-clipped drop acceptance and
an inaccessible long form. An unconditional layout now contributes the form's
scrollable height, with space below its action row. Wheel input can reach Apply
and save through a real pointer click. Ambiguous native-path edits are rejected
before copy/move enqueue or editor/command preference updates; unchanged paths
and checksum output basenames retain their captured native parents.

Strict all-target Clippy passed for the UI and application on both Windows and
Linux, including the final portable test fixture. Workspace Rust formatting,
source-size checks (981 handwritten files, each at most 100,000 bytes) and
`git diff --check` passed.

## Final Windows release

The optimized release build completed successfully in 6m 52s. The resulting
`_build/release/me.exe` is 42,806,272 bytes, SHA-256:

`C8A175A69D343D084E4EB857A898CD993CA36440C245FA42A2B32007B827C9C1`

The build and plugin packages were copied with hash verification into a new
private runtime. Its exact host hash was registered, and the existing redirected
Desktop shortcut was updated. Direct launch of the workspace release through
`--ui-smoke-test --snapshot ...` passed with exit 0 on native Windows DX12. The
captured main window was visually inspected: both panels and the full bottom
toolbar render, with no startup plugin error dialog. Existing real preferences
retained the hash recorded above.

Artifacts are under `_build/goal/commands/native-final/` (`acceptance.json`,
`deployment.log`, `native.out`, `native.err`, `main-window.png`). WGPU logs a
nonfatal `MAPPABLE_PRIMARY_BUFFERS` diagnostic for the discrete GPU; this startup
check does not establish GPU performance. Native viewer evidence remains in
the linked thirteen-stage viewer report. No runtime or build artifacts are
included in the source commit.
