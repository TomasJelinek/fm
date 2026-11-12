# Custom archive extension verification — 8 November 2026

Options → Archives now chooses among all eleven supported decoder families. Each
family retains its built-in extension checkboxes and has custom Add/Remove controls.
The shared core policy performs Unicode case folding and literal longest-suffix
matching; enumeration prepares filename hints on backend workers. Assignments do
not bypass trusted worker capabilities or change the OS association. Saved address
version 2 retains the exact decoder after a custom suffix is removed. Public
filesystem protocol 1 and independent plugin dependencies are unchanged.

The `CCfgPageArchivesAssoc` controls in the local Salamander reference were inspected
for the format/association relationship. No source or assets were copied.

## Completed checks

- Windows debug/test gate: 37 passed across core archive associations (3), custom
  suffixes (3), raw UTF-16 filename suffixes (1), shortcuts (15), existing archive
  preferences (5), custom archive preferences (3), archive bookmarks (4), and
  viewer preferences (3). Log: `_build/qa/archive-custom-windows-core-settings.log`.
- Linux core: the same portable archive cases and shortcuts passed, plus the raw
  non-UTF-8 basename / Unicode suffix test. Legacy association preferences passed.
  The first Linux custom preferences run exposed two test-fixture mistakes
  (bookmark native return path and optional empty map); both were fixed and the
  custom preferences cases subsequently passed on Windows. Log of the initial
  Linux gate: `_build/qa/archive-custom-linux-core-settings.log`.
- The rebuilt Windows Slint archive workflow passed Add for every decoder, global
  duplicate rejection, built-in conflict navigation, invalid draft preservation,
  Remove, reopening, and large-font/small-window bounds. The fixture now follows
  the shared footer's `Close` label. All 494 current UI unit tests also passed,
  including custom association persistence/dispatch presentation tests. Log:
  `_build/goal/archive-hotpaths/combined-ui-green.log`. The inspected validation
  screenshot is `_build/qa/archive-custom-ui/archives-custom-validation.bmp`.
- Direct runs of the compiled UI snapshot passed `parent_focus` (1),
  `dialog_resize` (1), and `copy_dialog` (5). The rebuilt metadata editor tests (2)
  now pass after correcting the obsolete unprepared `Apply` fixture expectation.
  Log: `_build/goal/archive-hotpaths/combined-ui-integrations.log`.
- Dependency captures were refreshed on native Windows and Linux and inventories
  regenerated. Counts remain 490 packages, 458 names and 60 direct package/version
  records. Existing `unicase` 2.10.0 now lists core and engine as direct dependants.
  Its pinned MIT license was inspected; no package/version/feature was added.
- A final Windows focused gate passed custom suffix policy (3), raw UTF-16 suffix
  handling (1), archive bookmarks including distinct decoder identity (5), and
  custom archive preferences (3). This includes the expanded raw suffix window
  for Unicode case equivalents whose original UTF-8 spelling is wider than the
  canonical suffix. Log: `_build/qa/drive-information-windows-tests.log`.
- Filesystem-host discovery/capability routing tests (4) passed, including explicit
  decoder availability and exact provider selection. Log:
  `_build/qa/archive-custom-catalog-windows.log`. Strict all-target core/common/
  native/engine Clippy passed on Windows and Linux in
  `_build/qa/drive-information-windows-clippy.log` and
  `_build/goal/find-refactor/backend-strict2.log`.
- The final Linux common gate passed archive bookmarks (5), custom archive
  preferences (3), and viewer preferences (3). The earlier fixture failures are
  resolved on both platforms. Log: `_build/goal/find-refactor/linux-common.log`.
  The Linux UI unit suite also passed 490 tests in
  `_build/goal/find-refactor/linux-ui.log`.

## Remaining qualification

The shared UI rebuild, focused backend gates and Windows/Linux custom settings
round trips are complete. Packaging and a deployed application restart remain
outside this source-workflow evidence.

The software-renderer screenshot verifies control layout and interaction only;
it does not establish native multilingual shaping. Decoder content support is
unchanged; custom suffix selection does not introduce a new parser or prove that
arbitrary renamed content is valid for the chosen decoder.
