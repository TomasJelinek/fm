# Bottom button alignment

The user requests vertical centering of the bottom-bar buttons. Inspection of
the native screenshot and Slint 1.18.1 layout implementation confirms that the
fixed-height icons are top-aligned inside FunctionButton's horizontal layout,
while text centers independently.

Keep the existing button heights and icon sizes. Set
`cross-axis-alignment: center` on that layout so icons and labels share the
vertical center in Desktop and Touch, including large fonts. Native colors and
image aspect ratios remain intact. This is a presentation-only follow-up to
the fully verified native-controls baseline `2de9d14`.

Subsequent user requests extend this increment: remove file/folder-row tooltips,
restore a single command row with the entire cwd, avoid enclosing inserted paths
in apostrophes, and diagnose apparently stuck folder sizes. Keep control/drive
tooltips. Actual desktop-token probes of drivers and the complete System32 batch
finish; the observed mismatch is a finished partial result drawn like an active
scan. Distinguish those states without changing the healthy scheduler. Reject
the separately observed Windows semantic-icon solid-black placeholder narrowly.

- [x] Add a rendered native-pixel/icon-and-label bounding-box regression and
  observe the expected failure before changing the product layout.
- [x] Apply the one-line layout fix and verify Desktop/Touch/large-font output,
  native RGB/aspect ratios and relevant footer input/tooltip regressions.
- [x] Verify single-row full-cwd geometry/scrolling, silent file/folder hover,
  finished-partial size styling, known Windows placeholder fallback, and literal
  shell insertion using real OS-separated shell/argv roundtrip tests.
- [x] Run focused checks, strict Clippy, app builds and source guards on Windows
  and Linux. Inspect the current native Windows screenshot and launch the app.
- [x] Update the design/verification record and prepare the reviewed follow-up
  for the explicitly requested commit and push.

Final integration uses a normal push to origin/main followed by comparison of
the local and remote revision; the resulting commit is reported in the chat.
