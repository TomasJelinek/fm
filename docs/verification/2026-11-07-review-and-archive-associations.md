# Review fixes and archive association qualification

This pass reviewed the F5 destination dialog and its surrounding queue, plugin,
cache and input boundaries, then added the requested archive associations and
blank-tab-strip gesture. It does not complete the remaining Move/extraction work.

## Behavior and review findings

- Existing-entry mutations wait for all previously admitted copies, including
  native-drop reservations. Failed, cancelled or incomplete predecessors prevent
  a later mutation from silently deleting an uncopied source. Later copies do not
  extend that fence. See the [queue contract](../superpowers/specs/2026-11-07-copy-predecessor-fence.md).
- Rejected large clipboard-copy payloads are retired through the backend. The
  new default F5 action respects a previously saved conflicting user binding.
- Viewer discovery and loading validate every bounded local package object,
  including sibling/subdirectory dependencies. Plugin trust is not weakened for
  writable development directories.
- Shared thumbnail stores coordinate transactions across processes and detect
  changed or torn generation records. See the [cache review](2026-11-07-plugin-cache-review.md).
- Native archive names remain lossless; synthesized ZIP parent labels retain the
  decoder's CP437 display text. Independent archive tests remain in the plugin.
- Viewer scrolling converts coordinates correctly at non-unit DPI. Main-window
  and viewer shortcuts share the same physical-digit and AltGr input policy.
- Double-clicking unused tab-strip space creates a tab for that panel. Existing
  tab clicks and non-left buttons preserve their original actions.
- Options lists all 57 known archive associations, with 51 ZIP-container suffixes.
  User documents/installers are opt-in. Available handlers, saved choices and
  regular-file identity govern both activation and the archive entrance badge.
  See the [format table and current limits](../ARCHIVE_FORMATS.md).
- The Options sidebar scrolls at large fonts in small windows, keeping the Done
  button reachable. Topic navigation supports Up/Down and Left/Right.

## Regression-test corrections

Full UI checks also exposed old test assumptions: F5 was still described as
unavailable, cancellation was expected in the job title rather than its state,
and popup dismissal was inferred from one pixel that could match the underlying
panel. Tests now assert the actual copy draft and focus restoration, terminal job
state and cancellation commands, and popup absence respectively. These corrections
do not relax the corresponding runtime behavior.

## Verification

- Windows core, settings codec, filesystem host and application composition tests
  passed. Engine verification passed 188 unit and 59 integration tests; its
  explicit performance fixture remains ignored in ordinary test runs.
- Windows UI logic passed 362 tests. Native UI integration targets cover archive
  settings/badges, tabs, copy drafts, cancellation, focus, keyboard settings,
  dialogs, pointer input and scrolling; all targets passed after correcting the
  stale assertions above (the explicit performance benchmark remains ignored).
  Default, filtered, font-size-32 and
  small-window settings renders are retained in ignored
  `_build/qa/archive-associations/`.
- Linux core, engine and filesystem-host tests passed. Linux UI logic passed 359
  tests, plus the archive badge, archive settings, copy dialog, Options and tab
  integration targets. Archive preference validation passed on both platforms.
- Independent archive-worker tests passed on Windows and Linux. Targeted viewer
  input, plugin loading, cache and native store tests also passed on both systems.
- The Windows release was rebuilt with a linker map for the
  [executable-size investigation](2026-11-07-release-executable-size.md), without
  changing optimization profiles, graphics backends or third-party dependencies.
  A fresh private installation passed SHA-256 readback verification; its main
  window is visible and responsive, with empty startup error output.
- Final workspace formatting, all-target strict Clippy, whitespace checks and
  the 100,000-byte source-size guard passed on Windows (708 checked source files).

Generated files, linker maps, captured images, test fixtures, worker staging and
runtime logs remain under `_build/` and are not committed. No debug/build artifacts
are included in the source changes.
