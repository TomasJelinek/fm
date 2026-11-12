# Make File List

Implement the user's requested Salamander workflow, adapted to Midnight Explorer's
Unicode, provider, asynchronous backend and native-window contracts.

## Reference behavior

Reviewed local Salamander `src/mainwnd4.cpp::CMainWindow::MakeFileList`,
`src/fileswn6.cpp::CFilesWindow::MakeFileList`, `src/dialogs.cpp::CFileListDialog`,
`src/execute.cpp::MakeFileListExpArray`, `src/keyboard.cpp` and
`src/lang/lang.rc::IDD_FILELIST`.

- Ctrl+M / Make File List uses selected displayed entries, or the focused real
  entry when nothing is selected. Never include `..`; preserve panel order.
- The editable per-entry template defaults to `$(FileName)$(CRLF)` and has
  history, variable insertion and useful presets.
- Support FileName, FileNamePart, FileExtension, FileSize, FileDate, FileTime,
  FileAttributes, FileDOSName, Drive, Path, DOSPath, CR/LF/CRLF/TAB.
  Variable names and `:max` are case-insensitive; `$$` is literal `$` and
  `$[ENV]` expands an explicitly referenced environment value on the backend.
- Fixed `:N` widths follow Salamander's explicit padding/truncation semantics,
  using complete Unicode characters. `:max` measures the captured set first.
  Formatting never changes operation identities or imposes MAX_PATH.
- Output targets: Clipboard, Internal Viewer, File; file output supports append.
  Successful output clears the captured selection only if the pane/selection
  revision still matches, never a selection made while generation was running.

## Approach and integration

A resizable owner-modal window captures an immutable provider-neutral
listing/selection snapshot. It has the Make File List icon and title, no independent
taskbar item, and a preview that fills the remaining client area as it grows.
Cancel/Escape restores the owner. This supersedes the earlier modeless choice. Template parsing is pure core code; enumeration,
formatting, environment expansion, disk/clipboard access and cleanup run in bounded
backend services. Native and archive rows use their own identities; expanded-tree
rows use occurrence identities. No archive extraction or directory recursion is
necessary to export cached listing metadata.

The synchronous legacy implementation would block the UI and a separate export
plugin would duplicate existing host services. A core format contract plus backend
job and small UI presenter fits the existing application interfaces.

Generated UTF-8 is streamed to the existing private anonymous/delete-on-close spool
service. Preview and clipboard have explicit memory budgets; File and Viewer do
not require the complete output in RAM. The viewer receives an immutable readable
content capability and uses the existing basic viewer plugin, including Escape.
No generated content is executed or passed to an external file association.

File Browse uses the system Save dialog. Existing destinations require a separate
confirmation with exact captured metadata. The backend retains the destination
pin through confirmation, rejects drift/links and publishes through the existing
verified transfer contract. Append stages existing bytes plus new output, avoiding
partial in-place append. Retained originals and durability warnings are reported
honestly according to that contract. Cancellation and late events cannot retarget
an export or resurrect a closed window.

Template history and last accepted output choices are persisted; failed/cancelled
drafts do not replace saved choices. Unknown provider metadata stays unknown;
DOS aliases are platform-specific and are never fabricated. Platform adapters stay
in separate files. No new third-party library is required.

## Verification

Template grammar/Unicode/width/error cases; exact selection and ordering including
filtered/duplicate-name archive/tree rows; cancellation and stale revisions;
streaming output, create/append/overwrite, changed destination, links and failed
writes; native picker cancellation; clipboard/viewer failures; keyboard/menu
dispatch and 320×200/touch layout. Run focused tests, strict checks, debug build
and an isolated native startup smoke, documenting platform qualification honestly.
