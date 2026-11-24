# Make File List services

The feature is a host service, not a new plugin. Existing viewer plugins consume
the generated text through the normal content-reader contract; their public ABI
and independent projects are unchanged.

## Core contracts

`FileListCapture` retains shared listing, order and selection snapshots. Native
selection uses raw paths, archive selection opaque entry IDs, and expanded trees
occurrence IDs. Its directory string is presentation data, never I/O authority.
The backend resolves the captured order and excludes `.` and `..` defensively.

`FileListTemplate` is a pure parser/formatter over `FileListRecord`. Environment
values and metadata are supplied explicitly by a backend worker. It never reads
the filesystem or process environment itself. When a template uses `:max`, the
worker measures every captured row before streaming rows; without `:max` it skips
that pass, so a preview validates only the rows it shows while clipboard, viewer and
file output still validate every row. Templates are bounded to 64 KiB
(`MAX_FILE_LIST_TEMPLATE_BYTES`) and expanded individual rows to 8 MiB
(`MAX_FILE_LIST_ROW_BYTES`); there is no fixed total file-list length limit.

`FileListRequest` has a session ID and monotonically increasing generation.
`RunFileList`, `ConfirmFileList` and `CancelFileList` are backend commands.
Preview events and terminal results carry both identities. The UI ignores stale
results and defers large snapshot disposal to the existing backend retirement
mechanism. Rejected admission must not cancel a previously accepted preview.

## Output lifetime and safety

The controller (`file_list_actor.rs`) admits two workers and at most four retained
session jobs; the UI opens one session at a time. Preview is capped at 12 rows/16 KiB.
Full output streams through the existing private spool in 256 KiB chunks
(`spool::MAX_IO`). `GeneratedTextSource` exposes immutable length and bounded random
reads; it does not reveal a native path.

Clipboard output carries its own cancellation lease. The native clipboard worker
checks it before reads and again before mutation, including after native session
initialization. Clipboard text has no size budget: the worker reserves its memory
fallibly and a text that does not fit fails with an error, separately from file and
viewer output. Cancelling a dialog cancels its lease even if the command queue is full.

The generated viewer adapter constructs a `CapturedFile` from an immutable
`ContentSource`, assigns a distinct version, and never reopens a fabricated display
filename. The UI hands the launch to the application's normal viewer manager after
releasing the presenter borrow. A one-shot startup result acknowledges the first
rendered content frame, not merely window construction. Capture, plugin and initial
rendering failures, or closing before readiness, leave the draft and selection intact.
Both request ID and generation must still match when that asynchronous result arrives.
Launch also requires the accepted startup plugin choices. While those settings are
loading, the draft remains available for retry instead of default-enabling a viewer.

File generation does not hold the shared mutation gate. Publication acquires it
cancellably afterward. `save_file_list` retains an opaque destination pin and the
generated spool through explicit overwrite/append confirmation. Consent binds the
path, target revision and append mode. Append releases the validated prefix reader
at EOF before native replacement; the target pin remains checked before publication.
`VerifiedFileCopy` verifies the staged contents before publishing them. A confirmed
Replace or Append keeps the displaced original in a recovery copy, which the
completion notice names (acceptance: `panels_file_list`).

`TransferDirectory::stage_file_preserving_target` is an optional native capability
whose default is Unsupported. It initializes existing-output security and supported
metadata while staging is empty. Windows and Linux implement it separately; an
unsupported metadata form produces an error. No content-only fallback may weaken
the existing output's access controls. Ordinary transfer behavior is unchanged.
Windows preserves owner, group, effective DACL and supported integrity labels;
the replacement protects the captured DACL from later parent inheritance. Privileged
audit-SACL preservation is outside this contract. Alternate streams and unsupported
storage/security metadata are refused instead of silently discarded.

On Windows the original's attributes decide how the replacement is stored, not whether
the confirmed replacement happens (`transfer_preservation.rs`, tests in
`transfer_preservation_tests.rs`):

- Read-only, hidden, system and not-content-indexed are kept exactly. Archive is kept
  too, and Windows sets it again when the replacement is put in place, as for any
  changed file. A stage that took the read-only attribute is still removed when the
  replacement is given up.
- Compressed (NTFS compression) is a storage detail: the replacement is compressed
  exactly when the original was, whatever the folder gives new files
  (`FSCTL_SET_COMPRESSION` on the empty stage).
- Encrypted (EFS): the stage is created encrypted, and the replacement is refused with
  that reason when it could not be. A plain original in a folder that encrypts new files
  gets an encrypted replacement, as any new file there.
- Sparse: the replacement is written as ordinary data.
- Offline and recall-on-open or recall-on-data-access files (cloud files whose contents
  may not be on this computer) are refused with that reason: replacing one downloads
  nothing, but its original would be kept for recovery only as a placeholder. Links and
  other reparse points are refused as before, and so is any other attribute, which the
  message names by value.

A Copy's own Replace keeps no metadata of the original and replaces such files too
(`a_copy_replaces_a_compressed_read_only_file`).

## Preferences and UI

Preferences (schema 26 and later) store the last 30 unique accepted templates
(`MAX_FILE_LIST_HISTORY`), output kind, native output path and append choice. Older
settings migrate to defaults. Failed
or cancelled drafts do not overwrite accepted choices. Output completed while
Options is open saves independently without persisting other previewed settings;
Options Cancel preserves that accepted history.

The system Save picker only chooses a path. It does not create a file or authorize
overwriting. The dialog is a modal popup embedded in the main window; it owns
editable fields and presentation only, with no separate taskbar item. Cancel
begins closing immediately; if the
backend queue is full, the retained draft retries cancellation and cannot admit a
later Replace/Append confirmation.
Input capture is immutable, and successful output clears selection only when the
original panel/view/selection revision still matches.

The line template is the dialogs' common edit (`ThemedLineEdit` with its inline clear
×); Down or Alt+Down in it opens the recent templates (`HistoryList`), and Escape
closes an open list before the dialog. Presets, Insert variable and Output are the
common dropdown (`CompactPicker`) in `file-list-dialog.slint`; the column's layout is
in [dialog ergonomics](DIALOG_ERGONOMICS.md#make-file-list-layout) (UI tests
`file_list_layout`).

The modal admission follows the reference file manager's Make File List: OK gates
output and Cancel exports nothing, a folder is refused as the destination, replacing
an existing file is asked first, and the selected entries or the focused one are
listed. This implementation adds pinned, versioned overwrite/append consent and
recoverable publication. No reference code is copied.

See [user workflow](../MAKE_FILE_LIST.md) and
[design](../superpowers/specs/2026-11-09-make-file-list-design.md).
