# Options OK / Cancel transaction

Options now stages changes across all pages. OK validates retained forms and
submits one asynchronous settings save; the dialog closes after the backend
acknowledges success. Cancel, Escape and X restore the original options, including
appearance previews, shortcut assignments, archive associations, plugin choices,
history settings and staged Custom Commands. Storage or validation errors keep
the session open. Closing also restores focus to the file panel.

Unrelated session saves and shutdown project the accepted settings into their
snapshots. They cannot accidentally persist an unaccepted theme or other preview.
Navigation, window geometry and newly executed command history remain intact.
Late color-validation and native-picker results cannot revive cancelled drafts.

Custom Commands has an Update command action to stage an individual form before
editing another command. The global OK/Cancel decision includes those staged
forms and command reordering. Explicit thumbnail-cache cleanup remains an
immediate maintenance action, identified in its tooltip.

## Verification

Evidence directory: `_build/goal/options-transaction/`.

- Initial regression tests failed the old behavior (preview settings were sent
  to storage before acceptance; no OK button existed), recorded in `red.log`.
- Presenter preferences checks: **59 passed**, including seven new transaction
  cases. They cover rollback, save failures/retries, stale acknowledgements,
  late initial loading, queue refusal, independent session saves, history and
  asynchronous validation cancellation (`unit-tests.log`).
- The UI regression exercises real pointer and keyboard dispatch for Cancel,
  Escape, X and Enter/OK; validation of forms on hidden pages; error-page focus;
  asynchronous save completion; and rejection of validation after cancellation.
- **61 UI checks passed** across Options, keyboard, archive/editor/plugin pages,
  Custom Commands, dragging, themes, fonts, tooltips and small-window layouts.
  See `integrations-final.log` and `layout-followup.log`: the former's remaining
  font test assumed the old F10 menu layout; its corrected real-input category
  selection passes in the latter, together with tooltip checks. A hidden shortcut
  capture no longer intercepts F1 after OK; the regression verifies panel focus
  and the rendered help with the newly assigned shortcut.
- Visually reviewed `snapshots/options-preview.bmp`. Footer actions remain
  accessible at 320×200, both spacing modes and large fonts in the layout tests.
- Strict UI Clippy, workspace rustfmt, `git diff --check`, and the 100000-byte
  handwritten source cap passed (1164 source files).
- The final Windows debug build passed. Native `--ui-smoke-test` with registered
  viewer, filesystem and thumbnail plugins exited 0 with empty stderr. The real
  preferences SHA256 was unchanged:
  `BCF7388CC3C3E2742FD86FEBF6F91A3EDAC29FB312FDFC6BC054492BCBBAB053`.

Debug executable SHA256:
`5A1F7BCDB5B056FF42D7ECAA2099457EE2D0147DEFF74AD264253B10DC8090F0`.
Its protected plugin runtime is
`C:/Users/tjelinek/MidnightExplorer-20261108-031759-52f96323`.
See `build-final.log`, `deploy.log` and `app-acceptance.txt`. This is Windows
debug verification; no new release build or native Linux/macOS run was performed.

This change adds no dependency and changes no plugin API. Platform I/O remains
in the existing settings backend. The specification is
[Options transactions](../superpowers/specs/2026-11-08-options-transaction.md).
