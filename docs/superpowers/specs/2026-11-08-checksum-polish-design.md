# Checksum window polish

The checksum window should prioritize inspecting results and copying hashes.
The user requested a thorough UI pass; existing checksum algorithms, captured
selection, async execution and safe manifest publication remain the contract.

The current window always exposes four rows of settings, two unrelated selectors
and four equally stretched actions. Status follows every digest column, so errors
can be offscreen. Save has two different meanings with the same label.

Salamander's `src/plugins/checksum/lang/lang.rc::IDD_CALCULATE` and
`dialogs.cpp::CCalculateDialog` provide the useful reference: results occupy most
of the window, configuration is separate, and stop/save are explicit actions.
No source or artwork is copied.

## Approach

Merely tightening spacing would retain the workflow ambiguity. A wizard would
obstruct a modeless task that already starts immediately. Use a results-first
tool window with Results, Options, Save manifest and Details pages instead.
This is an implementation choice within the requested UI pass.

- Direct page buttons at ordinary sizes; one labeled page selector when narrow.
- Algorithm/recursion controls belong to Options and remain frozen during work.
- Results show the chosen full hash in a horizontally scrollable column and retain
  all algorithms through the chooser and row context copy. File, bytes and status
  precede the hash, making failures visible immediately. Measured monospace width
  keeps the column readable without huge fixed-width allocations.
- Verification rows show a green vector tick beside Verified and a red vector
  cross beside Mismatch, Missing, Source changed or Read error. Calculated,
  Skipped and Cancelled retain neutral text without a success/failure glyph.
  Icons follow the typed backend result state, never the existence of a hash.
  Status text and accessible icon labels retain the meaning without relying on
  color; light/dark colors, font scaling and compact rows remain supported.
- Copy explicitly names the chosen available algorithm. Full-row copy remains in
  the context menu and gains Ctrl+Shift+C; Ctrl+C copies the chosen digest.
- Only calculated algorithms appear in the output chooser. Shared selection
  controls are used by Results and Save; changing it never recalculates files.
- Save manifest opens a labeled destination form. Its Save action is disabled
  until output, destination and required partial-result acknowledgement are valid.
  Destination and acknowledgement survive page switches; a new generation resets
  acknowledgement. Errors and partial-discovery counts remain visible.
- One status strip, context-relevant actions, a pinned Close button, and reachable
  Stop while running. Escape/native X retain responsive cancellation semantics.
- Retain 320x200 minimum, scroll oversized page content, bound line-edit height,
  preserve theme/font scaling and keyboard/screen-reader access.
  Very short result regions use single-line rows and omit the header when it
  would leave no complete row. Output selectors explicitly synchronize across
  pages and generation/model replacement after native combo selection.

No new dependencies or platform-specific code. Rust presentation only maps bounded
algorithm choices; hashing, clipboard formatting and writes stay on the backend.

## Verification

Render actual components before/after. Assert results space, no clipped footer
actions, all theme/density/page combinations, large fonts, cancellation, invalid
and partial export states, keyboard/context copying and stale-generation refusal.
Run focused Windows/Linux tests, scoped lint, source-size checks and debug build.
