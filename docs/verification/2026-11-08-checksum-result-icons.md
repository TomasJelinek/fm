# Checksum verification result icons

Result rows now show a green vector tick for `Verified` and a red vector cross
for `Mismatch`, `Missing`, `SourceChanged` and `ReadError`. `Calculated`, `Skipped`
and `Cancelled` remain neutral. Status text remains visible; icons also expose
accessible Passed/Failed labels. Colors adapt to light/dark schemes and the small
icons fit within existing row heights. No backend checksum logic changed.

Reference checked: Salamander `src/plugins/checksum/dialogs.cpp`, verification
list's `LVIF_IMAGE` branch around line 1935. It distinguishes completed row icons
from unfinished rows. No source or artwork was copied; the check SVG is locally
drawn and the cross reuses the application's existing close vector.

Validation on Windows:

- Regression first failed because `Verified` had no Passed indicator (`red.log`).
- All 12 checksum window/polish tests passed (`green.log`).
- Backend-event-to-UI coverage checks all eight row states and prevents calculated,
  skipped or cancelled rows from appearing verified.
- Pixel checks verify green success and red failure in light and dark themes.
- Both rendered theme snapshots were visually inspected. Existing tests also
  cover minimum sizes, theme/density combinations, large fonts, copying,
  cancellation and manifest save confirmation.
- Scoped formatting/diff checks and the source-size check passed.
- Debug application build and native startup smoke passed (exit 0, empty stderr,
  user preferences unchanged). The matching protected plugin runtime was
  registered for `_build/debug/me.exe`. Release was not rebuilt.

Artifacts: `_build/qa/checksum-result-icons/`. These are presentation tests;
hashing, file identity validation and native publication remain on their existing
backend paths. No dependencies, platform-specific implementation or plugin ABI
were changed.
