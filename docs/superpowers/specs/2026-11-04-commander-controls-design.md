# Commander controls and desktop integration

Later density update: the user's smaller Touch request supersedes this original
44px baseline; current40px metrics are in
[navigation and appearance](2026-11-04-navigation-and-appearance-design.md).

Scope approved in chat: extend the current read-only UI. File copy/move/delete,
rename, editing and creation remain disabled until their safe handlers exist.
The external command line is an explicit user-controlled shell action.

## Behavior

- Bottom F1–F10 bar follows Salamander: Help, Rename, View, Edit, Copy, Move,
  Create directory, Delete, User menu, Menu. Unsupported actions are visibly
  disabled, including keyboard dispatch; no fake successful operation.
- Command editor submits on Enter/Run to a system terminal, with the active
  pane's cwd captured at submission. The picker on its left offers PowerShell/cmd
  on Windows, Terminal/Terminal(sudo) on POSIX. No process-wide cwd changes. Commands run
  only on explicit submit, with bounded launch capacity and background reaping.
- Ctrl+[ and Ctrl+] insert left/right pane paths at the caret, replacing any
  selection. Paths are quoted for the selected shell, byte-exact for Unicode;
  PowerShell ASCII/curly apostrophes and POSIX apostrophes are escaped. No command
  launches on insertion. These shortcuts work from panels and the editor.
- A nonempty draft is bound to the grammar used at its first edit/insertion.
  Late loaded defaults cannot reinterpret it; cross-grammar selection is rejected
  with clear-line guidance. POSIX Standard/Elevated changes keep the same grammar
  but still require explicit submission. Native Windows consoles must have their
  own keyboard/screen buffers, even when Midnight Explorer diagnostic output is redirected.
- `..` is a synthetic first row outside real file IDs, sort snapshots, selection
  and counts. It is omitted at a root; Enter/double-click goes up; Space never
  selects it; Insert can advance. Rollback/refresh preserve parent cursor state.
- Left click focuses its row/pane; Ctrl+left click toggles multiselection. An
  unmodified left double click or Enter enters a directory or opens a regular
  file through its host association. Explicitly activated directory links stay
  internal. Selection, right click, hover and refresh never launch files.
  Pointer activation captures listing/entry identity across press and both
  clicks; sorting or refresh cannot substitute a different row. Focus and cursor
  updates preserve Slint delegates so the actual double click remains detectable.
- Alt+Enter in a file panel opens selected items' properties, otherwise the
  focused real item. It takes precedence over ordinary Enter, also with Ctrl+Alt
  on Czech layouts. A bare focused `..` has no Properties action. The command
  editor does not route Alt+Enter to panel properties.
- Each panel gets its own horizontal location-button bar: real volumes and
  supported Home/Desktop/Documents/Downloads locations. Drive buttons restore
  that pane's remembered directory; special-location buttons open their exact
  location. No invented Network/cloud entries. The closest location is marked.
- Compact-control update: real plain-text tooltips explain navigation, shell
  choice, command cwd and shortcuts. The shell selector is a small terminal
  icon immediately before the editor; descriptive choices remain in its popup.
  Remove the persistent keyboard footer, command placeholder help and ACTIVE
  word; retain pane decoration, actual status/errors and explicit F1 help.
  The cwd remains visible in a compact read-only prompt, with the full path in
  a tooltip. Mouse density uses22px rows/24px controls/14px icons and narrower
  spacing; Touch retains44px targets. File/folder icons are vertically centered.
  Options uses explicit close-on-click-outside behavior so interior clicks and
  nested theme/density selection do not dismiss it.
  Drive buttons show cached Windows volume label/free space in tooltips; Linux
  shows free space. Expanded drive choices include root/letter, native label
  when present and free space, without widening the closed toolbar control.
  Separate two-slot backend metadata lookup refreshes about every 45 seconds;
  loading/unavailable states never masquerade as zero capacity.
- Shift+drive-letter switches the focused file panel using metadata supplied by
  Windows. Editors keep ordinary shifted typing; Linux supplies no drive letters.
- File/folder icons come from host associations/themes on backend workers, with
  bounded type-keyed caches and a font-independent generated fallback. Windows
  uses synthetic names/attributes, never extracts icons from listed executables,
  shortcuts or custom folders. Linux initially accepts bounded trusted theme PNGs.
- Preserve Slint's virtualized ListView/Flickable inertia for touch/touchpad and
  its existing short wheel animation (~180ms). Do not add a competing content-y
  animation. Keyboard reveal and metadata/icon refresh must not snap the viewport.
- The active pane has an accented header and a subtle frame; selection and
  cursor remain distinct. Command-editor focus retains the last active pane.
- Ctrl+Shift+S starts a background size scan of every directory in the active
  panel, ignoring selection. The same shortcut cancels it. Navigation cancels
  obsolete work; navigation and new independent requests remain available.
  Sizes arrive progressively by navigation/scan/entry identity, without a UI
  traversal or a full-list sort for every progress event.
- Clicking a column heading toggles ascending/descending order; changing columns
  starts ascending. A dedicated `Resort` command sorts the committed snapshot
  on a worker, preserving item IDs and completed folder measurements. Only
  finished, error-free totals participate as known sizes; incomplete totals stay
  unknown. Refresh requests fresh filesystem data and invalidates measurements.
  Failed navigation retains the previous backend snapshot for UI rollback.
- Directory sizes are logical lengths of ordinary files in a changing tree,
  excluding links/special files. Running/cancelled/error results are visibly
  partial. Checked arithmetic rejects false totals on overflow. A 16 MiB pending
  directory-path budget bounds wide-tree work; exhaustion reports partial
  results. It does not impose a maximum length on an individual path.
- Size scans use separately budgeted backend coordinators with cooperative
  cancellation and progress throttled to 100 ms per root. A path-based read-only
  provider cannot promise atomic directory identity against concurrent renames
  or link replacement. Future mutations require stronger opened-handle contracts.
- The storage-aware extension uses at most four active root workers per scan and
  one shared actual-I/O budget across the two panes: proven local SSD work costs
  one of four slots; HDD, network and unknown work requires all four slots.
  Storage classification has its own single bounded gate so a blocked SSD read
  cannot consume every slot needed to classify a smaller job. Known network
  classification uses local mount/drive information before remote path probing.
  Long scans yield fairly at callback boundaries (100ms /256 entries) within the
  active bounded batch; this is one active I/O stream on HDD/network, not parallel
  seeks. Blocking OS calls cannot be forcibly interrupted.

## Interfaces and safety

Core adds separate `IconService`, `TerminalService`, `PropertyService`,
`LocationService` and `OpenService`, aggregated as `DesktopServices`. Existing `FileSystem` tests
and contracts stay unchanged. Native implementations live in per-platform files.
Icons cross threads as bounded owned straight-RGBA pixels; Slint Image creation
and caching stay on the UI thread. No UI filesystem/image loading or native calls.

Desktop commands/results use request IDs and the existing bounded message bridge.
Native properties may occupy their own bounded workers; they cannot starve
navigation, icon work or terminal launches. Fallback properties are read-only
backend metadata, never an implicit file launch. Terminal command text is never
logged; stdin text is intentionally shell code, inserted filenames are literals.
File activation has its own four-call budget and at most sixteen tracked desktop
helpers; it cannot occupy navigation capacity. Windows uses Unicode
`ShellExecuteExW(open)` with separate filename/parameter fields. Linux uses an
absolute system `xdg-open` or `gio open` executable with literal native argv and
null standard streams. Helper failures are reported and children reaped on the
backend. Late directory-link results require matching request/pane/navigation
identity. Missing or special files fail without launching an alternative target.
Literal insertion is restricted to recognized standalone argument boundaries.
Ambiguous or quoted editor positions are rejected without changing the command;
the application does not attempt to parse arbitrary shell programs. Deliberate
user code such as eval/sh -c can reinterpret arguments, as in an ordinary shell.

## Automatic filesystem refresh

- Each supported OS implements the separate core `WatchService` contract in its
  own native crate/file. Windows uses native directory notifications, Linux uses
  inotify through pinned notify 8.2.1. A future macOS adapter must implement the
  same contract. Native callbacks report rescan hints, not filesystem truth.
- Watch only the immediate contents of the two visible directories. Create,
  delete, rename, content/metadata changes, overflow and errors become a bounded
  dirty signal. Ignore read/access events so listing/polling cannot feed back.
- Debounce for 1 second of quiet; cap a continuously changing burst at 3 seconds.
  Keep only the latest target and a dirty counter per pane, never an unbounded
  queue of event paths. Two separately bounded backend workers own native guard
  registration, cleanup, polling and data disposal. UI performs no watch syscalls.
- Every 15 seconds, compare native names, kinds, lengths and modification times
  on a worker against the accepted listing. Unchanged polls do not refresh. This
  catches missing notifications on network filesystems and native overflow/error
  gaps. It is a consistency fallback, not a real-time delivery guarantee.
- The app acknowledges only listings actually accepted by the UI via
  `PaneCommitted(VisibleDirectory)`, sharing the immutable entries with Arc.
  A 100 ms short input-side retry prevents bounded command-queue saturation from
  permanently losing this acknowledgement. A queued listing ignored as stale
  must not replace the visible snapshot or the watched directory.
- Refresh hints carry pane and navigation identity. UI rejects obsolete hints or
  hints received while user navigation is pending. Polling/change hints retry if
  a refresh failed or was ignored; successful refresh updates the watch target.
- Same-location refresh stages replacement data behind the still-visible rows,
  then applies it atomically. Preserve selection, cursor, wheel-scrolled viewport,
  active pane and command-editor text/focus. Do not recompute recursive folder
  sizes automatically; fresh filesystem data invalidates their earlier estimates.
- notify 8.2.1 has Windows notification cases where overflow/read errors may not
  reach the callback. Periodic polling is required even after native registration
  succeeded; it must not be disabled based on a successful watch call.

## Appearance and preferences

- Remove the entire top Midnight Explorer/About branding row. About stays in F10 Menu. Options
  lives beside Run and in the menu; no replacement full-width header is added.
- Themes: Blue(default), neutral Dark, Light. One Slint Theme global owns semantic
  surfaces/text/focus/selection/error/disabled colors and the std-widget scheme.
- Density: Desktop/mouse compact; Touch uses at least44 logical px for principal
  hit targets and file rows. Shared metrics also drive scrolling/reveal/hit tests.
- Preview is immediate and does not change pane paths, selection, operation state
  or command editing. Backend SettingsService persists typed version3 preferences
  under Windows LocalAppData or Linux XDG_CONFIG_HOME/~/.config.
- Missing config yields defaults. Strict bounded decoding rejects malformed data,
  unknown fields/versions and non-regular/symlink/FIFO targets. Atomic save uses
  an exclusively created temp, complete writes/sync and safe same-directory
  publication; existing settings survive errors. Linux files are private0600.
- Monotonic PreferencesRevision protects local edits against late load/save
  responses. Engine serializes native calls and coalesces pending writes to the
  latest state. Failures retain preview and expose retry. On close, the window
  ends first, pending event payloads go to backend reclamation, then the latest
  accepted save receives a bounded2s grace; a stuck OS call has no false guarantee.

## Salamander reference trace

Sources are inspected as behavioral reference, without copying implementation:

| Feature | Reference |
| --- | --- |
| F1–F10 | `help/src/hh/salamand/shortcuts_keyboard.htm:41` |
| Ctrl brackets / selection replacement | `src/editwnd.cpp:955`, `:337`; `src/fileswn0.cpp:1174` |
| Command cwd | `src/editwnd.cpp:482` |
| Alt+Enter before execute | `src/fileswn0.cpp:172`, `:1161` |
| Mouse double click / file-directory activation | `src/fileswn9.cpp:877–900`; `src/fileswn2.cpp:53` |
| Selected-or-focused Properties | `src/mainwnd3.cpp:3517`; `src/shellsup.cpp:1625` |
| Parent Properties exclusion | `src/shellsup.cpp:1255` |
| Fixed left/right drive bars | `src/toolbar6.cpp:137`, `:190`, `:208` |
| Drives and special locations | `src/drivelst.cpp:1962`, `:2480`; `help/.../windows_drivebar.htm:22` |
| Active-panel directory sizes | `help/src/hh/salamand/advwork_calcsize.htm:17` (upstream Ctrl+Shift+F10; this app Ctrl+Shift+S) |
| Automatic/network refresh | `help/src/hh/salamand/configuration_drive.htm:20`, `:31` (notification gaps on network drives) |

Intentional adaptations: lossless Unicode shell arguments (simple tokens bare,
special syntax escaped in double quotes) instead of Salamander's ANSI text,
own remembered paths per pane, supported physical special locations,
and read-only-stage capability gating. Inspect relevant Salamander sources for
every further feature and record differences instead of assuming parity.
Mouse double-click activation requires no modifiers; Ctrl+click remains selection
and Alt+Enter remains properties. This intentionally omits Salamander's special
Alt-double-click branch until a separately defined action is implemented.

## Verification

Test shell literal Unicode/punctuation, UTF-8 caret boundaries/selection, no submit
on insertion, frozen cwd, duplicate Enter suppression, parent row identity and
rollback, Alt+Enter precedence/selection, fixed per-pane location targets,
remembered drives and Shift shortcuts not stealing editor text, bounded icon
cache/fallback/decoding/native handle ownership, async desktop isolation, and
actual Slint focus/shortcut/scroll integration. Run Windows/Linux tests, strict
Clippy, formatting, 100000-byte guards, native window smoke, and relaunch visibly
outside the sandbox after successful validation.
