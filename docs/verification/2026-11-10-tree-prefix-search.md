# Inline tree prefix search and archive extension sorting

## Scope and design

Typing in a native or archive panel now searches loaded inline branches using
the same Unicode filename-prefix matcher as the ordinary panel. The backend
builds ordered match positions and grapheme-safe highlight boundaries with the
immutable tree projection. Query changes do not enumerate directories or read
file contents. Persistent filters and sibling sorting compose with the query;
quick-filter mode keeps unmatched ancestors as unselectable navigation context.

The presenter retains one desired query and at most one in-flight projection
request. Escape, rapid typing and Options preview/Cancel supersede stale results.
Transient queue pressure retries without an error flash. Semantic failures remain
visible. Match navigation uses the backend index; pending and zero-result search
cannot activate a leftover cursor. Resolved selection requests are separate from
successfully applied authority, avoiding a permanent wait after rejected input.

Insert retains one exact matching occurrence while clearing search, then toggles
it and advances only after the empty projection and preceding selection settle.
New queries, navigation and disappearing rows cancel this intent. Publication
validation precedes any saved-caret or pending-intent mutation.

Integration review also found that extension sorting used native paths even for
archive members, whose native paths are deliberately empty. Shared sort-key
preparation and comparison now borrow the original raw filename extension.
Archives use their UTF-8 basenames without interpreting host separators or
display captions; native entries retain host path semantics. Folders remain
first and unknown extensions remain last in both directions.

No new dependency, plugin ABI or filesystem-operation authority is introduced.
See the [plan](../superpowers/plans/2026-11-10-tree-prefix-search.md) and
[search contract](../superpowers/specs/2026-11-05-panel-search-and-metadata-design.md).
Salamander `src/fileswn0.cpp` informed Escape, Insert and bounded match movement;
the user's Unicode and filter requirements remain authoritative.

## Recorded verification

Evidence is under `_build/qa/tree-prefix-search/`.

- Backend regressions failed before implementation in `backend/red-{core,engine,
  integration,flat}.log`. Full Windows core/engine packages then passed 946 tests,
  with two intentionally ignored, in `backend/green-packages.log`.
- Archive sorting had two expected failing regressions before its fix. The final
  core unit, locale sorting and tree suites passed 79 tests in
  `backend/green-archive-sort.log`. Strict core Clippy passed.
- Linux backend qualification passed 23 core tree/range tests, 78 engine tree
  units, 20 search units and 26 search/controller/filter integration tests.
  These initial results precede the archive extension follow-up; final affected
  qualification is recorded separately below.
- Twenty focused presenter regressions passed in `ui/green-final-presenter.log`.
  They exercised latest-query ordering, transient failures,
  ancestor/zero-match activation, saved-caret continuity and out-of-order
  selection receipts. Insert and stale-publication regressions were added during
  independent review before final qualification. Independent source review
  cleared that version. A later preference-parity regression additionally checks
  that enabling quick filtering moves a hidden caret to the first match while
  preserving a caret already on a valid match.
- The first real Linux UI bridge run passed the ordinary panel search, four tree
  filter tests and four of five new tree-prefix tests. Options Cancel exposed a
  fixture omission: it waited only for engine events, while the app's 100 ms
  timer also retries pending search/preferences. The fixture now drives that
  production presenter path on idle timeout, under a five-second deadline.
  This changes no production timer or provider behavior.
- Broad Windows UI qualification ran 94 suite summaries: 983 tests passed, six
  failed and one was ignored. Five failed fixtures contained stale expectations;
  the sixth was the new tree-search fixture's missing timer and then a mutable
  borrow incorrectly held across refresh. The mutable guard now ends before the
  callback-bearing refresh, matching the existing fixture/event path. The
  navigation fixture now completes its captured listings before pixel stability
  checks, so an intentionally animated loading indicator is not mistaken for a
  filename tooltip; the strict no-tooltip assertion is unchanged.
  These failures are retained in `ui/final-package.log`; final reruns follow below.
- The broad run exposed stale test assumptions from earlier
  UI changes. `bulk_selection` now asserts Ctrl+Shift+A selects files only and
  locates the top Options gear by accessibility instead of an old bottom-bar
  coordinate. `dialog_drag` expects the current About title while retaining its
  movement, large-font and Escape checks. `draft_shell` and `focus` retain their
  literal-text, Unicode, shell and caret checks with the required trailing native
  directory separator. `native_controls` and `tooltips` locate semantic Home and
  drive roles instead of assuming positions that predate the Network menu item.

The corrected quick-filter parity assertion failed on both platforms with the
hidden caret still on entry 7 rather than first match 11. The fix only applies to
an acknowledged nonempty filtered query; it preserves an already matching caret.
Independent review cleared the final condition and its Insert/empty-query fences.
Windows' final affected union passed 715 tests: 694 UI units and 21 integration
tests. The corresponding logs are `ui/final-affected.log`,
`ui/final-affected-remaining.log` and `ui/final-native-controls.log`; the first also
retains the earlier loading-animation fixture failure before its isolated rerun.

Final Linux core sorting/tree checks passed 31 tests. The subsequent UI run passed
the full 689-test unit suite and the tree search/filter bridges. Its two failing
targets were the then-unfixed navigation loading fixture (now separately passing)
and a genuine Options footer overflow, described below. Both failed targets were
subsequently requalified; this report distinguishes those reruns from the broad
run rather than replacing its recorded result.

The initial strict core/engine all-target lint run found existing dead-code
warnings in unfinished tree-operation consumers and one test-only redundant
clone. The redundant clone was corrected; the full workspace is not represented
as warning-free by this scoped report.

## Options footer follow-up

`linux/layout-repro.log` identifies the exact failing stage: Options at 320 by
200, touch/font32 after shrinking. Cancel starts at x=167 and has width 157,
extending four pixels beyond the right window edge. The original single-row
action layout lets Linux font-dependent button minima overflow its available
width. The accepted fix reuses the shared `DialogFooter`, whose dismissal control
stays anchored while its action region can scroll. Reset, OK, initial focus,
validation cancellation and the save-time dismissal lock remain required.

The final footer source passed 38 tests in 11 integration suites on each of
Windows and Linux (`ui/final-footer.log`, `linux/final-footer.log`). These include
all six dialog movement/resize/close tests, initial compact Options at font32,
validation cancellation, disabled actions during save, and restored physical
Cancel after the save lock clears. Options transactions, tooltips, copy/metadata
dialogs and both tree search/filter bridges remain green. Independent review
also confirmed the existing ready/default-OK and loading/default-Cancel focus;
pinned Slint reveals focused descendants in ancestor scroll views.

Strict selected UI/core all-target lint passed with `--no-deps -- -D warnings`
(`ui/final-scoped-clippy.log`). The dependency build still reports 13 known engine
warnings; this is not a warning-free whole-workspace claim. Focused Rust
formatting, whitespace checks and the 100,000-byte source limit passed for 1,611
handwritten files.

## Final debug build and installed runtime

The rebuilt `_build/debug/me.exe` has SHA-256
`aba2ba71dff79ea99e3497e71c6573832d1b1cae015404089016b59ffd0d718c`.
Its identical protected copy is registered at
`C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe`. The ordinary build-path
executable passed `--smoke-test`, listing five drives and both fixture panels
(30 and 132 entries). Evidence is in `build.log`, `build-hash.json`,
`deployment.log` and `app-smoke/` beneath this report's artifact directory.

The unchanged HTML provider also passed the native host probe against this
installation: real pixels, Unicode, Find, default script denial, fractional-DPI
resize, fresh online/script context, return to offline and supervised close.
The rendered bitmap was visually inspected. The first invocation used a
noncanonical Windows installation path and was rejected before launch; passing
the canonical extended path, as the production installation resolver does,
corrected the test invocation without changing security checks.

`html-runtime-canonical/` records 22,325 ms to first content and 621 ms to close.
Because first rendering was unexpectedly slow, a repeat was measured:
`html-runtime-repeat/` records 937 ms and 381 ms respectively. Both passed.
These isolated debug observations do not identify the cause of the slow run or
establish release performance. The exact host probe SHA-256 is
`1764a34f42ba317780fbdfa2d48187b6e5317fe9c52c49a26e5dcb65706c899c`.
The provider package and its earlier actual Slint-window qualification remain
as documented in the [HTML report](2026-11-10-html-viewer.md).

The later [tree directory-size integration](2026-11-10-tree-folder-sizes.md)
records the next debug executable and registered startup. The hashes and runtime
measurements above remain the evidence for this prefix-search baseline.

## Limits

Software-window tests exercise actual Slint events, presenter callbacks and
engine publications with controlled native/archive providers. They do not prove
physical IME input, all desktop compositors or release scroll performance.
Search covers loaded rows only. Typed descendant operations, container expansion
and branch watch/refresh retention remain separate backlog work. This report
does not declare the entire inline-tree feature or broad application goal done.
