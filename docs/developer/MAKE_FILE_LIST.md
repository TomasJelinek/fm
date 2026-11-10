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
the filesystem or process environment itself. The worker measures `:max` widths
before streaming rows. Templates are bounded to 64 KiB and expanded individual
rows to 8 MiB; there is no fixed total file-list length limit.

`FileListRequest` has a session ID and monotonically increasing generation.
`RunFileList`, `ConfirmFileList` and `CancelFileList` are backend commands.
Preview events and terminal results carry both identities. The UI ignores stale
results and defers large snapshot disposal to the existing backend retirement
mechanism. Rejected admission must not cancel a previously accepted preview.

## Output lifetime and safety

The controller admits two workers and at most four retained session jobs. A single
UI session is currently exposed. Preview is capped at 12 rows/16 KiB. Full output
streams through the existing private spool in 256 KiB chunks. `GeneratedTextSource`
exposes immutable length and bounded random reads; it does not reveal a native path.

Clipboard output carries its own cancellation lease. The native clipboard worker
checks it before reads and again before mutation, including after native session
initialization. The existing 4 MiB clipboard budget remains separate from file and
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
`VerifiedFileCopy` verifies the staged contents before publishing them.

`TransferDirectory::stage_file_preserving_target` is an optional native capability
whose default is Unsupported. It initializes existing-output security and supported
metadata while staging is empty. Windows and Linux implement it separately; an
unsupported metadata form produces an error. No content-only fallback may weaken
the existing output's access controls. Ordinary transfer behavior is unchanged.
Windows preserves owner, group, effective DACL and supported integrity labels;
the replacement protects the captured DACL from later parent inheritance. Privileged
audit-SACL preservation is outside this contract. Alternate streams and unsupported
storage/security metadata are refused instead of silently discarded.

## Preferences and UI

Preferences schema 26 stores the last 30 unique accepted templates, output kind,
native output path and append choice. Older settings migrate to defaults. Failed
or cancelled drafts do not overwrite accepted choices. Output completed while
Options is open saves independently without persisting other previewed settings;
Options Cancel preserves that accepted history.

The system Save picker only chooses a path. It does not create a file or authorize
overwriting. The modeless native window owns editable fields and presentation only.
Input capture is immutable, and successful output clears selection only when the
original panel/view/selection revision still matches.

See [user workflow](../MAKE_FILE_LIST.md) and
[design](../superpowers/specs/2026-11-09-make-file-list-design.md).
