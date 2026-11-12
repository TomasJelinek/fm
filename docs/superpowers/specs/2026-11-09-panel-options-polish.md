# Panel feedback and Options refinements

Implement the reported layout/input defects together with the user's reset scope.
Shared components handle all panes and platforms; disk access and network discovery
remain in the backend. There are no new dependencies or preference format changes.

- Error notices reserve fixed narrow controls, letting diagnostics wrap/scroll
  across the remaining width. Copy is at the bottom left and dismiss at the top
  right. Compact notices retain both actions; clipboard receives the complete
  diagnostic without display truncation.
- Modified physical bracket keys resolve through the configurable shortcut map.
  Ctrl+[ and Ctrl+Shift+[ insert the left path; the right counterparts insert the
  right path. Localized key text and shifted braces do not break them. Plain typing
  and AltGr remain editor input. No command executes from path insertion.
- Options Reset to defaults follows the [transaction contract](2026-11-08-options-transaction.md):
  all Options settings, preserving tabs/history/hot paths/custom commands; OK/Cancel
  controls persistence and rollback.
- Filename colors use a visible, keyboard-accessible priority list. The first
  matching enabled rule wins. Existing reorder, edit, picker and transaction
  actions remain available. The entire page scrolls in short/narrow windows.
- Accepted Network navigation immediately shows its requested location and a
  loading indication, hiding stale native rows. The intent is presentation state,
  never a fabricated filesystem identity. Partial results and same-location
  refresh keep loading visible without shifting the list. Failure restores the
  committed location with its diagnostic. Native slow refreshes also show loading.

Validation uses input/render tests for 320×200 and wider windows, physical-key
tests on US/Czech key text, Options preservation/rollback/save tests and presenter
tests for queue rejection, partial results, failure, superseded navigation and
operation routing. Inspect rendered artifacts and run the actual debug application
with a private verified plugin runtime before presenting the result.

Implementation references: Salamander `src/editwnd.cpp` and `src/fileswn0.cpp`
for bracket keys, `src/cfgdlg.h` and `src/dialogs4.cpp` for ordered coloring,
`src/fileswn3.cpp` and `src/plugins/nethood/nethoodfs2.cpp` for loading feedback.
The UI remains shared Slint code, independent of the Windows-specific backend.
