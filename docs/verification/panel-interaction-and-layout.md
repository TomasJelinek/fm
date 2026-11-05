# Panel interaction and layout verification

Scope: the [interaction design](../superpowers/specs/2026-11-05-panel-interaction-layout-design.md).
Evidence and temporary executables live in `_build/qa/interaction`; they are not committed.

## Behavior and regressions

- Selection uses at most 32 pending presentation intents. The backend remains
  authoritative for counts, ranges and actions. No directory-wide UI scan occurs.
- Ctrl/Shift/Ctrl+Shift clicks cover toggle, replace-range and add-range. Streaming
  listings have a backend incremental identity index, with no range length cap.
- Deferred All/Invert acknowledgements preserve visible defaults and explicit
  exceptions until final rebase. Failed queues, stale replies, sorting, filtering,
  navigation rollback and newly arriving entries are regression cases.
- Real Slint input tests cover command focus, visual panel swaps and side-specific
  shortcuts, splitter drag/reset and extreme sizes, shrinking/scrolling tabs,
  stale breadcrumb gestures and copy, compact controls, and terminal split actions.
- Clipboard targets use current generation-bound identities. Empty terminal
  launches preserve command drafts/history, shell binding and active native cwd.
  Windows and Linux argv builders are tested separately without launching shells.
- The drive menu's `. As Other Panel` action is covered by real period-key and
  mouse events after a visual panel swap, including Unicode/pending paths and
  backend queue refusal. Enter tests cover both file association and folder entry.
- Windows flags include Compressed and extended flags; unknown bits are labelled
  Unknown without a numeric mask. Unix mode display keeps symbolic and octal modes.
- Light surfaces and active/selected colors are neutral gray. The Type column is
  absent; the existing type-extension sort shortcut remains available.

## Measurements

All timings are local debug measurements with 10,000 synthetic entries per pane.
They describe the named harness and are not a release-performance guarantee.
Normal tests assert behavior, not machine-dependent timing thresholds.

The original software-renderer harness used 24 gestures and an artificial 25 ms
acknowledgement delay. Before changes, none of 24 accepted gestures showed a mark
on the first update; callback median/p95 was 1.555/2.955 ms, visible-selection
461.325/554.893 ms and resize layout+render 186.856/215.994 ms. The visible timing
includes drawing before and after the delayed reply, not real backend latency.

After the initial implementation, all 24 gestures showed their mark immediately;
callback median/p95 was 1.138/2.535 ms, visible-selection 73.963/78.531 ms, and resize
93.587/98.189 ms. Forced geometry accounted for 0.970/1.183 ms versus raster drawing
92.544/97.199 ms. These intermediate timings precede the final renderer profile.

An idle native WGPU run before renderer optimization measured resize-request to
render submission at 246.367/292.739 ms median/p95 (32 samples). The rendering
callback span was 154.157/192.061 ms. A separate concurrent-compilation run is
excluded from comparisons. There is no synchronous screenshot/readback in this
benchmark; submission timing still excludes compositor presentation latency.

Measurements of the selected rendering profile on the same Windows machine,
with the Linux build explicitly suspended and no concurrent Windows build/test
workload (before the final menu addition and large-font editor adjustment):

| Metric (ms, median / p95) | Before | Optimized |
| --- | --- | --- |
| Selection callback, software harness | 1.555 / 2.955 | 0.447 / 0.504 |
| First frame after click, software | 188.814 / 252.367 | 22.116 / 26.027 |
| First visible selection, software | 461.325 / 554.893 | 22.153 / 26.054 |
| Resize layout + render, software | 186.856 / 215.994 | 32.373 / 34.181 |
| Resize request to render submission, native WGPU | 246.367 / 292.739 | 211.171 / 259.109 |
| Native rendering callback span | 154.157 / 192.061 | 115.549 / 136.691 |

All 24 final gestures show selection on the first update, before the delayed
backend acknowledgement. Final software resize geometry costs 0.147 / 0.186 ms;
rasterization costs 32.203 / 34.042 ms. The actual backend selection-state update,
measured separately without injected delay, costs 0.0036 / 0.0038 ms for 10,000
entries and 0.0202 / 0.0206 ms for 100,000 entries (100 samples each).

Native resize improves by about 14% in this debug harness, substantially less than
the software result. It remains above an interactive frame budget; this is not a
claim of smooth 60 Hz native resizing. The native notifier adds an extra renderer
flush, and the native desktop uses its own DPI, so native/software numbers must
not be compared as an equivalent-renderer benchmark. Release and compositor
presentation latency have not been qualified by these measurements.

Reproduce the benchmarks explicitly:

```text
cargo test -p fm-ui --test interaction_performance -- --ignored --nocapture
cargo run -p fm-ui --example native_resize
cargo test -p fm-engine --lib interaction_selection_cpu -- --ignored --nocapture
```

Native measurements require a desktop and must run without concurrent builds/tests.
The debug profile optimizes the Slint core, FemtoVG renderer, WGPU core/HAL and
text shaping/font libraries at level 2. Generated/UI code uses level 1 and
backend/application crates retain ordinary debug settings. Release keeps full
optimization. Changing these profiles requires a one-time dependency rebuild.

## Review

Independent review found two issues and prompted regressions: deferred selection
marks disappeared after an acknowledgement, and a narrow Bytes-mode pane clipped
its Modified column. Deferred defaults now follow backend membership semantics.
Size columns adapt to the available pane width; exact bytes remain copyable in
the status field. The follow-up selection review found no remaining blocker.

The gray active-row surface also exposed insufficient contrast in the old amber
pending-size color. Darkening that semantic color fixed the regression while
retaining the existing 4.5:1 contrast assertion.
Linux's wider native font metrics exposed an editor viewport below 300 px at
large font sizes. A 320 px editor reserve, where space permits, fixes it while
retaining the strict width assertion and the complete, pannable working path.

## Windows verification

- The integrated workspace run passed 549 tests with one contrast failure.
  After its correction, all 13 tests in the six affected UI targets passed.
  The final menu/Enter/layout check then passed all 238 tests in the UI library
  and six affected integration targets, including two new tests. Together these
  runs cover 552 distinct passing tests.
  Five explicitly opt-in tests remain ignored by ordinary workspace runs.
- The final backend lint cleanup was followed by all 100 engine unit tests.
  Strict workspace/all-target Clippy passed with warnings denied.
- The normal debug application and native benchmark example build successfully.
  Rustfmt and whitespace checks pass; all 223 handwritten source/script files
  are within the 100,000-byte limit.
- The normal Windows EXE completed its native UI smoke test with Unicode paths
  and system icons. Its SHA-256 was
  `1a431711fab3c8c788c5a7aaf2a8027bbcd3adc8a82b550a68d222cc38aa4723`.
  Existing preferences were unchanged by the test.
- Inspected native and software snapshots cover the two panel toolbars, drive
  chooser, contiguous breadcrumbs, command split button, Light desktop/touch
  geometry, the other-panel menu item and Options topics. Theme-transition
  screenshots wait for the native widget color animation to finish before
  assessing contrast.

## Linux verification

- Ubuntu 24.04 under WSL: the integrated workspace run passed 520 tests and
  identified the one large-font editor-width failure described above.
- With that fix and the final menu addition, all 235 tests in the affected UI
  library/integration targets pass: 221 unit tests, nine interaction tests and
  five other integration tests. Together the runs cover 523 distinct passing
  tests. The strict editor-width/full-path assertions remain unchanged.
- All 100 current engine unit tests pass after the final worker-message cleanup.
  Temporary inotify fixtures use WSL's native filesystem so the filesystem-watch
  tests exercise Linux events; compiler and application artifacts stay in `_build`.
- Strict workspace/all-target Clippy passes with warnings denied.
- The normal debug application builds and its headless native-filesystem smoke
  test passes (15 discovered volumes, 20 left entries and eight right entries).
  This is Linux backend/runtime verification, not a Linux desktop visual review.
  Product/test source hashes match before and after final verification.
