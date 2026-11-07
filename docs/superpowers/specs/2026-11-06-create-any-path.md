# F7 directory creation at an explicit path

F7 accepts a folder name, a relative path (including `.` and `..`), or an absolute
filesystem path. Relative paths start from the active panel directory captured
before the dialog; later navigation cannot change that base. Absolute targets may
be outside the panel, on another drive, on a UNC share, or across a Linux mount.
Missing intermediate directories are created; the final directory must be new.

Windows root-relative paths use the captured panel's drive or UNC share root.
Drive-relative paths such as `D:child` use the captured panel only when its drive
is D. A different drive's implicit current directory is ambiguous: the backend
returns an error asking for an absolute drive path. It never reads a process-wide
per-drive current directory to decide where to create. Device namespaces, ADS,
reserved Windows basenames, and embedded NUL remain invalid. There is no MAX_PATH
limit or lossy conversion of native names.

Creation runs entirely on mutation workers after explicit confirmation. The
prepared panel identity is revalidated even for an absolute target. Both native
adapters walk the supplied components using directory handles. They inspect each
intermediate before proceeding and reject symbolic links/reparse points; they do
not lexically cancel a component with a later `..`. Consequently `missing/../new`
creates `missing` before creating `new`, and `link/../new` fails at `link`.
An explicitly entered Linux path may cross mounts; recursive delete and metadata
operations retain their separate no-mount-crossing policy.

Empty inputs and targets ending in `.` or `..` (or a root alone) fail before any
creation. A trailing separator after an ordinary final name is allowed. Existing
files, directories, and links at the final name are never overwritten. Native
errors and cooperative cancellation retain completed intermediate creation and
report that partial outcome. Handles do not turn a live namespace into a snapshot;
Linux cannot stop another authorized process from moving an opened ancestor.

Completion refreshes the source panel without navigating to an external target.
Only a new direct child is a focus/reveal target; creating elsewhere preserves the
existing panel search and caret. Directory watches refresh other visible parents.
The empty-file command retains its separate basename-only contract.

Behavioral reference: Salamander `src/fileswn5.cpp`, `CFilesWindow::CreateDir`,
resolves the submitted path, creates missing parents, then creates the last
directory and sends change notifications without navigating the panel. This
project implements that behavior independently with Unicode native handles;
Salamander implementation code and its legacy path-length limits are not copied.

Verification uses fresh owned fixtures under `_build/`: absolute targets outside
the base, sibling paths, Unicode and long paths, intermediate creation around
`..`, links before `..`, cancellation, base replacement, final collisions,
Windows root/drive-relative forms, and engine/UI routing after navigation.
