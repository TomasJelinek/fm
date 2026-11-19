# Find and panel follow-up — 18 November 2026

The active checkout is `codex/post-merge-validation-20261118`, based on
published `main` at `544b48db`. This batch remains on the branch. No commit,
merge or push is recorded for the current batch.

## Requested and implemented behavior

- Text comparison uses the window close control and Esc, with the redundant
  toolbar Close button removed. Find supports next/previous, case, whole words,
  clear, result counts and centered matches; shortened paths expose tooltips.
- Text viewer Find preserves the real line prefix and indentation and centers
  the shaped match. Whole words and an inline clear control are implemented.
- Keyboard options align shortcut fields and adjacent action buttons across
  desktop, touch and large font sizes.
- Panel selection uses a distinct mark shape while retaining the theme color.
- Panel filter history sits beside the expression edit and preserves draft
  mode/case and clear semantics. The latest polish reuses the existing
  `ThemedLineEdit` clear control to remove the duplicate clear button. Its five
  affected runtime cases passed the final replay.

Implementation does not establish runtime acceptance. The evidence below is
scoped to the recorded Windows tests and source snapshots.

## Verified results

| Boundary | Latest completed evidence | Result |
| --- | --- | --- |
| Comparator Engine Find | `_build/compare-find-polish/arc-tests.jsonl` | 9 passed, 0 failed; includes the prior 8 cases |
| Viewer Engine Find | `_build/html-timeline-validation/viewer-find-replay.jsonl` | 19 passed, 0 failed after lint-fixture changes; same cases as the prior qualified run |
| Viewer search cursor | `_build/viewer-find-polish/cursor-final.log` | 4 passed, 0 failed |
| Real Basic and Source plug-ins | `_build/viewer-find-polish/native-final.log` | 1 passed, 0 failed |
| Viewer resident lifecycle | `_build/viewer-find-polish/resident.log` | 6 passed, 0 failed |
| Comparator/viewer UI library | `_build/viewer-find-polish/runtime-lib-result.json` and its case logs | 14 passed, 0 failed: comparator Find 3, viewer Find 3, text position 8 |
| Main UI consolidated scoped cases | Parent checkout `_build/qa/ui-options-panel-proof-20261118-v7-filter/handoff.json`, with referenced exit receipts and stdout logs | 25 unique selected cases: 5 latest filter passes and 20 retained unaffected passes; 0 latest failures |

The comparator Engine command was
`cargo test -p midnight-compare-engine --lib text::find::tests --locked --offline --jobs 1`.
Its original eight cases include Unicode overlapping folded matches, rapid
navigation, bounded discovery, cancellation, shaped placement and resize
behavior that preserves intentional scrolling away from a hit. The subsequent
`_build/compare-find-polish/arc-tests.jsonl` replay passed **9** Engine cases and
**3** comparator UI Find cases, with zero failures. The added Engine case checks
immutable Find snapshots and preservation of unread frames across state
changes. The previous eight Engine cases and three UI cases overlap these
results; reruns are not added together. Earlier five-, seven- and eight-case
Engine receipts retain their historical scope.

Viewer Find covers context loading, Unicode word boundaries and case folding,
ambiguous/synthetic anchors, cancellation and clear, retained indentation,
shaped RTL extent, initial/delayed viewport publication and bounded provider
calls on dense substrings. The real Basic/Source case exercises whole-word
exhaustion and the real row prefix against the existing trusted installation.
The UI library cases cover stale revisions and first-image-frame Find, queued
state reset, shared Find actions, responsive controls and position retries.

The 25-case main UI consolidation includes Keyboard geometry; panel selection,
theme and filter behavior; Viewer/Thumbnailer associations; tooltip ownership
and real folder-link lookup; Properties request arguments; transfer progress,
receipts and window ownership; real F5 Copy/F6 Move; and preference loading.
The earlier v6 result qualified the snapshot before the final clear-control
polish. The v7 filter replay passed all five affected cases (four filter layout
cases and one embedded-history case), with zero failures and clean compile
receipt. Its consolidation retains 20 unaffected earlier cases: this remains
25 unique cases, not 30 or a repeated full target. Root reviewed the focused
nonempty filter capture: one clear X precedes the history arrow.

### Comparator/viewer integration and latest replay

The initial integration run passed 18 of 22 cases. The repaired-fixture replay
passed three of the four failed selectors, leaving 21 distinct passes and one
comparator keyboard geometry failure at that point. Receipts:
`_build/viewer-find-polish/runtime-integration-result.json` and
`runtime-integration-retry-result.json` in the same directory.

The subsequent seven-case comparator resize replay completed with **6 passed,
1 failed**. The three comparator library cases, comparator keyboard/Esc case,
hidden Find spacing case and compact action geometry case passed. The Find
integration case failed at `tests/find.rs:271` on its full-path tooltip assertion,
after the match and resize checks passed. Evidence:
`_build/viewer-find-polish/runtime-resize-result.json`, `resize-runtime.log` and
the referenced per-case `test.log` files.

The final affected Find integration replay then passed **1**, failed **0**, using
real captured long Unicode paths in the repaired fixture. Only the fixture
changed after the six-of-seven resize run; production code did not change.
Log: `_build/compare-find-polish/tooltip-final-runtime/test.log`. This cleared that
content assertion; it did not establish popup containment, as the later
review below demonstrated. The production keyboard/Esc, hidden Find
spacing and resize behavior now have passing affected-case evidence. These
replays overlap the earlier cases and are not added together as new coverage.

Root visually reviewed Keyboard desktop captures at 12 and 32 px, the grayscale
selection marker, normal/narrow viewer Find, and comparator Find and tooltip
captures. Root also confirmed the corrected narrow comparator capture shows
the actual hit centered and visible. Visual review remains separate from the
automated assertions.

### Actual tooltip containment and shared helper replay

Root found that the earlier popup-origin assertion gave false assurance: the
popup `WindowItem` measured 18×12 while its 318×45 text extended outside the
360 px host. The corrected regression failed with exit 101 in
`_build/compare-find-polish/containment-red-corrected/test.log`; the recorded
text right edges were 418 and 598 px for the two headers.

The fix reuses the existing shared compiler adapter and adds weak post-event
observation and tooltip anchors in `midnight-ui-input`. Main UI delegates only
the anchor calculation to that helper. The GREEN compile finished successfully;
locked metadata is retained alongside it. All **12 exact runtime cases passed**
with zero failures in
`_build/compare-find-polish/containment-green/runtime-result.json` and its case
logs: four shared helper cases, the integration-module guard, three comparator
Find library cases, real Find with both header tooltips, keyboard/Esc and two
geometry cases. These overlap prior replays and are not summed.

In the corrected 360×320 host, the left tooltip starts at (104,38), measures
248×76, ends at x=352 and contains text ending at x=342. The right tooltip starts
at (8,38), measures 248×76, ends at x=256 and contains text ending at x=246.
Both popup bounds and actual text extents are now checked. The shared extraction
also passed all **4** Main UI tooltip cases in parent checkout
`_build/qa/ui-options-panel-proof-20261118-v8-tooltip-shared/result.json` and
`tooltip-suite.stdout.log`. Those checks are a separate affected boundary,
not four extra unique cases added to the 25-case consolidation.

The real Source plug-in 20,000-line global-position, Copy and preferences case
passed **1**, failed **0** in
`_build/viewer-find-polish/source-position-native.log`.
The dependency inventory records the pinned compiler/helper licenses and that
all **615** resolved package/version/source/checksum identities remain unchanged;
only direct-dependant edges changed. See `docs/THIRD_PARTY.md`.

### HTML timing evidence and pending work

The four-run native idle ABBA matrix completed on the bounded prebuilt
baseline/candidate pair. Parent checkout evidence:
`_build/qa/html-native-probe-win-v1/idle-abba-hidden-20261118-233931-3c3d181a/measurement-summary.json`.
First-open and burst-onset observations favored the candidate; reopen and wheel
results were mixed. The limited same-session sample supports no overall speed
claim. The earlier draw endpoint was the next native observer boundary, an upper
bound that includes intervening scheduling and work.

The example now adds bounded CPU timeline records and rendering-notifier
markers. Its **3** example tests passed with zero failures in
`_build/html-timeline-validation/tests.jsonl`; the example build completed
successfully in `build.jsonl` in the same directory. These qualify the example's
checks and build, not native performance behavior.

The source review in `_build/html-timeline-review-v1/REVIEW.txt` and its hashed
`evidence.json` establishes further measurement limits. In pinned Slint 1.18.1,
installing the notifier adds a clear-command flush and submission, so tracing
changes the submission path. `AfterRendering` occurs before `present_surface`;
CPU frame publication and native observer return also do not establish GPU or
display presentation. The current data cannot assign the long tails to CEF,
UI dispatch, surface acquisition or presentation. A same-binary trace-OFF→ON
native control has a Build lease but no completed result at this checkpoint.
No overall speedup or production bottleneck is claimed.

### Targeted static validation and pending integration

Strict Clippy for the six targeted packages with all targets and warnings denied
completed successfully with no compiler errors in
`_build/html-timeline-validation/clippy-clean.jsonl` and
`clippy-clean.stderr.log`. Earlier diagnosed lint failures remain in
`clippy.jsonl` and `clippy-final.jsonl` beside them; the clean replay supersedes
their failure status for that targeted scope. The Viewer Find replay after the
lint-fixture changes passed the same **19** cases with zero failures in
`_build/html-timeline-validation/viewer-find-replay.jsonl`, overlapping the prior
19-case result.

Root independently verified source-size checks for **2,708** files and a clean
diff whitespace check. The 64-file Media overlay remains unapplied; Architect
is refreshing its worker preimage. Full suite, packaging and broader platform
qualification remain open.

### Retained directory-link transfer evidence

`cargo test -p midnight-engine --lib directory_link_move_tests --offline --jobs 1`
passed **10**, failed **0**. Log:
`_build/reviewed-transfer-regression/recheck.log`.
The native Windows same-volume dangling-junction Move test passed **1**, failed
**0** with
`cargo test -p midnight-platform-windows --lib same_volume_follow_moves_a_dangling_junction_without_qualifying_its_target --offline --jobs 1`.
Log: `_build/reviewed-transfer-regression/windows-native.log`.
These narrow passes do not qualify all Copy/Move workflows or Linux behavior.

## Remaining acceptance and limits

- Complete and review the same-binary HTML trace-OFF→ON control before using
  the instrumented timeline diagnostically; Media integration/qualification
  remains separately owned and open.
- These are Windows software-rendered tests with separately identified native
  boundaries. Properties verification records actual Alt+Enter routing through
  the production Engine into a native-service argument recorder; it does not
  establish that the operating system property sheet was displayed.
- The full suite, packaged executable, Linux behavior and the remaining backlog
  are not qualified by this checkpoint. The earlier screenshot follow-up and
  all 54 retained requirements remain open in `ACTIVE_WORK.md`.
