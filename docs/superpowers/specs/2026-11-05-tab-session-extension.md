# Tab session restoration and middle-click closing

Persist each panel's ordered native directory paths and active tab index through
the existing backend preferences service. Each panel remains bounded to 32 tabs.
Reuse the bookmark codec's lossless Unix-byte/Windows-UTF-16 representation;
display strings never become filesystem authority. Directory rows, selections,
scan state, navigation history, and runtime callback identities are not stored.

The preferences schema advances to version 6. Versions 1–5 remain readable and
start with no saved tab session. Malformed sessions, invalid active indices,
relative/NUL paths, and more than 32 locations fail validation without truncation.

On startup, restore untouched panels after the asynchronous preferences read.
Explicit `--left` and `--right` paths override only their corresponding panels.
Tab edits or navigation accepted before the read finishes take precedence over
the late saved session. Restored tabs receive fresh runtime identities; only each
panel's active location is queried. Rejected navigation leaves the current panel
intact and retains the restoration request for a later retry.

Tab creation, closing, selection, and accepted path changes update the saved
session. Writes remain revisioned and atomic through the existing backend queue;
failed reads continue to prevent overwriting unknown existing preferences.

Middle-click closes the pointed tab without selecting it first. The final tab
stays open. The pressed tab identity and model revision must still match at
release, so a reordered or replaced tab cannot be closed by a stale mouse press.

Version 6 also stores each panel's view mode as `detailed`, `brief`, or
`thumbnails`. Older preferences default both panels to Detailed. A view-mode
choice made before settings arrive overrides the saved choice only for that
panel, including an explicit choice of the default Detailed mode.

The renamed application saves Windows preferences below
`LocalAppData/Midnight Explorer` and Linux preferences below
`XDG_CONFIG_HOME/midnight-explorer` (or `~/.config/midnight-explorer`). When that
file is absent, the existing `FM` / `fm` location is read through the same native
regular-file and no-follow checks. An existing current file, including an invalid
one, prevents fallback. Loading never writes; subsequent atomic saves target only
the new location and preserve the legacy file unchanged.
