# Mouse navigation and explicit file activation

Scope: left click moves the cursor/activates its pane, Ctrl+left click toggles
multiselection, unmodified left double click enters a directory or opens a regular file
through the host association. Enter uses the same activation rules. Parent `..`
stays internal navigation; selection, right click, hover, listing and refresh
never launch anything. F3 remains the future built-in viewer.

Known directories navigate directly through existing asynchronous Navigate.
Files and symlinks use OpenEntry with a frozen raw path and request/pane/navigation
identity. Separate OpenService implementations classify explicit symlink targets
on backend workers: linked directories navigate internally, linked regular files
open via the desktop. Missing and special files return contextual errors.
Late directory results cannot override newer user navigation.
Frozen row tokens contain navigation intent, displayed snapshot generation and
entry identity. Refresh, resort and delegate reuse cannot silently substitute
another target during a click sequence. Cursor/selection notifications affect
only the changed rows; resetting the model on focus would destroy Slint's
double-click tracking. Delayed link results preserve the active pane if the
user has since focused the other pane.

Windows uses Unicode ShellExecuteExW with the `open` verb and distinct path/
parameter fields. Linux uses a trusted absolute desktop opener and one literal
absolute path argument; no shell source, elevation or filename interpolation.
All native metadata/launch calls and helper reaping stay on bounded backend
capacity, independent of pane listings, properties and terminal launches.
Paths remain native and dynamically sized; host failures are reported without
truncation or an alternative path. Explicit activation may run programs according
to host associations; it is a user action, never an automatic browsing operation.

Verification: actual Slint press/release/double-click sequences; both panes,
single/no launch, Ctrl selection, parent offset, sort/refresh identity, right
button/no launch, files/directories/links, Unicode/metacharacters/long paths,
queue saturation and stale results. Native tests inspect arguments/validation
without opening arbitrary user files. Backend tests gate the native service and
verify navigation remains responsive and helper failures reach the UI.

Implementation ownership: root core/engine/docs/app wiring; UI worker fm-ui;
native worker per-platform OpenService files. Reference Salamander behavior is
audited before implementation and recorded in the accompanying plan/evidence.
Salamander references: `D:\experiments\salamander-main\src\fileswn9.cpp:877–900`
focuses the pane/cursor and calls Execute; `src/fileswn2.cpp:53` separates disk
files and directories. This implementation requires unmodified double click,
keeps Ctrl+click for selection, and uses dynamic native paths instead of ANSI
or MAX_PATH buffers. No reference source or assets are copied.
