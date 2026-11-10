# Transfer dialog options

The F5 Copy and F6 Move dialogs capture their source selection and destination
before confirmation. Navigation, sorting or a clipboard change cannot replace that
captured input. The default remains content-only copying with independent complete
destination verification.

“Preserve exact modified time” is an unchecked, per-dialog choice. It is limited to
at most 64 distinct native regular files. Archive members, known folders and larger
selections cannot enable it. Cached selection/index facts determine availability;
unknown native/clipboard kinds still require authoritative backend preflight.
The UI never opens paths or enumerates selected rows to decide availability.

An explicit choice maps to `Command::EnqueueTransferWithMetadata` with
`TransferMetadataPolicy::RequireModifiedTime`, retaining the captured Selected or
Clipboard input and Copy/Move intent. An unchecked option uses the
existing commands. Unavailable required metadata is an error; it does not silently
fall back to content-only behavior. A new dialog starts unchecked; queue refusal
retains the current draft and option for retry. No preference or settings schema
is introduced.

The backend preflights source kinds, root count, required capabilities and staging
rights before content writes. Exact value/precision support is verified on each
unpublished destination stage after content verification; unsupported precision
fails rather than rounding. Earlier completed copies can remain if a later file
fails. Move removes each source only after that file's successful content and
metadata proof; earlier completed moves remain after a later failure or cancel.
Failure before that file's temporary staging leaves its original name. Every
pre-deletion failure/cancel after staging attempts to restore the original name
without replacing a new winner. Successful Move leaves no retained source copy
or staging directory. An exceptional failed restore or cleanup reports its exact
temporary location in Jobs. There is no recovery opt-in, catalogue or journal.
See [native transfer guarantees](TRANSFERS.md).

This exposes the first required-metadata slice. It does not imply that permissions,
ownership, ACLs, all timestamp fields or arbitrary provider metadata are preserved.
