# Application actions and archive preferences layout

## Changes

- Options is an icon button at the right of the shared top toolbar. The next
  button displays the original application moon and opens About, including the
  application version. Both remain reachable when the action strip scrolls.
- The bottom strip contains F1–F12 and Jobs; it no longer duplicates Options.
- Custom archive extensions and their add control align with the built-in
  extension list. Their child coordinates are explicit: Slint otherwise centers
  the intrinsic-width controls inside their full-width containers.

## Evidence

The pre-change archive geometry test failed with the custom extension at
`x = 699.5`, compared with the built-in checkbox at `x = 400`. The toolbar test
found Options in the old bottom row at `y = 792`. Logs are under
`_build/qa/archive-alignment/{red,toolbar-red}.log`.

The first combined debug gate passed all 36 tests: archive preferences (5),
commander edges (9), commander controls (3), Find windows (5), shared toolbar
(10), and small-window layout (4). Log:
`_build/qa/archive-alignment/green.log`.

Rendered archive layouts cover 1340×820 at font 13, 640×640 at font 24, and
320×500 at font 13. Root inspection confirmed left-aligned extension chips and
the add button. Toolbar checks exercise both 1340- and 320-pixel widths, opening
Options and About by pointer, the version label, and Escape closing About.

Review caught a further presentation issue: passing the moon as a fallback
action icon applied monochrome tinting. It now uses the untinted image slot.
The final combined gate passes all 42 tests, including the six path-elision
regressions, after that correction. Its log and refreshed captures are in
`_build/qa/path-elision/`. Scoped rustfmt and whitespace checks pass; the source
size check covers 1,332 handwritten files, all within the 100,000-byte limit.
The debug application and `native_find` example build successfully. The native
Find probe passes, and the rebuilt application starts with the registered private
viewer/filesystem/thumbnail runtime. Its owned-folder smoke check exits 0,
produces `startup.png`, reports no standard error, and leaves the user's saved
preferences unchanged (`_build/qa/path-elision/smoke-result.json`). The original
running checkout application was closed normally, under the user's prior
authorization, to release its executable before rebuilding.

Verified `_build/debug/me.exe` SHA-256:
`3C35FBF423BD60DED74CD08E14254CD6FDE806386FDF3D461668CFD8225B9B20`.
The build still reports the existing unused inline-tree foundation warnings;
this record does not claim a clean workspace-wide Clippy or CI run.
