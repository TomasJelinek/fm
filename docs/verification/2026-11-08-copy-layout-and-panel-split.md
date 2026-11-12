# Copy layout and persistent panel widths

Verified on Windows x64 with the debug build on 2026-11-08.

The Copy/Move confirmation previously clipped the four-line source summary at
its initial 300px height. A rendering regression reproduced the supplied image;
the corrected initial height displays the source path above the destination.
Desktop, touch and enlarged-font snapshots were inspected. At 320×200 the source
area scrolls while the destination and action buttons remain reachable.

The central splitter now stores the normal width ratio in preferences schema 23.
Tests cover legacy defaults, strict validation, restart restoration at a different
window size, refresh during a drag, one save on release, keyboard adjustment,
double-click centering, temporary Ctrl+F11 zoom, delayed/failed settings reads,
queue refusal and independent Options cancellation. Storage uses the existing
backend preferences worker; moving the pointer does not write settings.

Validation:

- 64 presenter/preferences unit tests passed.
- 9 splitter, comparator and viewer preference codec tests passed.
- 30 UI integration tests passed across `splitter_preferences`,
  `interaction_controls`, `commander_tools`, `commander_edges`, `copy_dialog`
  and `dialog_resize`.
- 2 native Windows startup tests passed: the first frame already uses the saved
  split, theme and monitor geometry; a failed preference read uses defaults and
  cannot overwrite the old settings.
- Debug application build, scoped rustfmt/diff checks and the source-size check
  passed (1,294 handwritten source files, each at most 100,000 bytes).
- The packaged application launch check exited successfully with empty stderr,
  produced a screenshot and left the user preferences file unchanged. Existing
  viewer, archive and thumbnail plugins remain in the protected runtime.

Executable: `_build/debug/me.exe`, 84,134,912 bytes.
SHA-256: `bb52b9452af4175c6b1e0127c0b94d383c39d843613821ccf8bb0f243a01b00e`.
Logs and screenshots are under `_build/qa/splitter/` and
`_build/qa/copy-format/`; generated artifacts are not committed.

These are targeted Windows/debug checks, not a new full-workspace or release
validation. The build still reports the existing unused engine/tree warnings.
