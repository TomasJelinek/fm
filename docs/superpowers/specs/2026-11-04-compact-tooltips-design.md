# Compact controls and contextual tooltips

Later density update: the user's smaller Touch request supersedes this original
44px baseline; current40px metrics are in
[navigation and appearance](2026-11-04-navigation-and-appearance-design.md).

The user requests more space for files and command entry: replace persistent
instruction text with tooltips, use a small icon selector before the command
edit, and show volume names/free space in drive tooltips and expanded choices.

Keep Slint's built-in Tooltip lifecycle with plain Text content, accessibility
labels, explicit F1 help, real status/error feedback and readable action labels.
Remove the permanent keyboard-help footer, command placeholder and redundant
ACTIVE text; active-pane decoration remains. Keep a compact read-only cwd prompt
beside the editor and the full cwd in its tooltip.
The shell picker is first in the command row and directly precedes the editor;
its icon/chevron opens descriptive choices and its tooltip names the current
shell, privilege mode and cwd. Preserve draft grammar binding, explicit Enter
execution, keyboard focus and 44px touch targets. No shell launch on selection.

Drive controls remain compact when closed. The expanded menu shows the root/
drive letter, native Windows label when present, and available bytes formatted
as free space. Toolbar tooltips show the Windows label and free space; Linux
and future macOS do not invent drive letters or labels. Missing/loading/failed
capacity is explicit, never a false zero. Unicode metadata is plain text and
does not alter native navigation identities. Special-location tooltips explain
their actual path; they are not additional physical drives.

Root discovery stays independent of slow metadata. New `VolumeService::volume_label`
defaults to `Ok(None)`; Windows implements it in its own file using dynamic
UTF-16 buffers. A separate bounded backend controller queries label and capacity,
publishes `VolumeDetailsUpdated { volume_id, root, details }`, caches and refreshes
details about every 45 seconds, and suppresses obsolete discovery results. Native
workers retain their permits until real calls return. UI hover reads cached
values only. Unavailable/network storage must not block listing or input.

Windows capacity uses the same dynamic native root resolution followed by
quota-aware `GetDiskFreeSpaceExW`, replacing the previous fixed-buffer fs2 path.
It preserves missing-path errors and suppresses media dialogs on the worker
thread, restoring the prior thread mode afterward. Cache expiration has its own
deadline so a query finishing just after startup still refreshes after 45 seconds.
An exact ordered `(id, PathBuf)` revision cancels an open drive picker even when
different native paths have identical lossy Unicode display. Special aliases
such as Home include their real path in the expanded choice/tooltip.

Approach: retain existing models and callbacks, with a compact reusable popup
picker whose expanded width is independent of its button. A full-width text
ComboBox wastes command space; a narrow stock dropdown can clip required disk
details. Custom popup rows retain keyboard and accessibility behavior. Existing
F1 help remains available without reserving persistent workspace.

Follow-up corrections: center all file/folder icons inside their row height.
Desktop uses22px rows,24px controls,14px icons,6px window padding and4px gaps;
Touch now uses40px controls/rows. Desktop drive
buttons use46px captions such as `C:`; special locations use24px icon buttons
with full accessible tooltip labels, while Touch retains descriptive captions.
The current folder is visible in full beside the editor in a single command row,
separate from executable command text. There is no fixed prompt-width cap or
ellipsis; extreme paths pan horizontally while preserving editor space.
File/folder rows no longer show hover tooltips; controls and drives retain them.
Options explicitly uses
`close-on-click-outside` so interior clicks and nested picker selections work;
Escape and Done still dismiss it. Actual pointer regression covers
Dark→Touch→Light, continued interaction, reopening and Done.

Native QA exposed a font-dependent dropdown arrow; replace it with a vector
chevron and center it/the terminal icon in the compact button.

Reference inspected: Salamander `src/toolbar6.cpp:241` dispatches drive-bar
tooltips; `src/drivelst.cpp:2561` and `:2621` combine volume name/free space.
This implementation moves all such I/O to the backend and uses lossless Unicode
instead of its ANSI/MAX_PATH buffers. No reference code/assets are copied.

Verification covers real popup keyboard/pointer operation and focus, plain
Unicode tooltip data, missing/stale metadata, blocked metadata isolation,
bounded refresh, volume selection identity, dark/light/touch layouts, source
size, formatting, Windows/Linux tests/Clippy/build/smoke and native screenshot.
macOS adapter work remains a later milestone.
