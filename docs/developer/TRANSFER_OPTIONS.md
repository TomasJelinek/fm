# Transfer dialog options

The F5 Copy and F6 Move dialogs capture their source selection and destination
before confirmation. Navigation, sorting or a clipboard change cannot replace that
captured input. The default is content-only copying with independent destination
verification.

## Preserve times and attributes

“Preserve times and attributes” is a per-dialog check box, offered for every Copy and
Move (native items, archive members, inline-tree and clipboard captures) of any size.
It starts as Options › General › File operations › **Preserve times and attributes in
Copy and Move** says, off by default; changing it in one dialog changes only that job.
It is saved as `copy_preserve_metadata` (settings schema 43,
[settings files](../SETTINGS_FILES.md)). It replaces the former “Preserve exact modified
time” check box, which was limited to 64 regular files and never saved, so a settings
file of an earlier schema starts with the new option off.

A checked option maps to `TransferMetadataPolicy::PreserveTimesAndAttributes`
(`Command::EnqueueTransferWithMetadata`, `CopyOptionsRequest`, `CapturedTransferRequest`
or `ArchiveCopyRequest::metadata`). The backend carries it as the job's frozen
`preserve` choice (`CopyInput::preserve`, `Job::set_preserve_metadata`); it has no root
bound and no preflight, so a selection of up to 1,000,000 items streams as without it
([selection sizes](TRANSFERS.md#selection-sizes)). What it preserves, in which order and
with which limits is in [preserved times and attributes](TRANSFERS.md#preserved-times-and-attributes).
Paste and drag and drop, which have no dialog, copy content only.

The default is off, as in Open Salamander, whose comparable options (Preserve directory
times; Preserve Archive, Compressed and Encrypted attributes) are off by default: the
option also copies access and creation times, folder times and Read-only, which change
what later programs and Copy jobs can do with the copies, so users opt in.

## Required modified time

`TransferMetadataPolicy::RequireModifiedTime` remains an engine contract that the dialog
no longer offers: at most 64 distinct native regular-file roots
(`MAX_REQUIRED_METADATA_ROOTS`; more are refused before any source is captured with
`REQUIRED_METADATA_ROOTS_REFUSAL`, for a Copy and a Move alike, never copied
content-only instead), each pinned and staged before anything is copied, and an
unsupported precision fails the file instead of rounding. Archive members are refused.
See [required modified time](TRANSFERS.md#required-modified-time).

## Other choices

The dialog's Links choice is described in [links](TRANSFERS.md#links-in-copy-and-move);
“Wait for earlier transfers” maps to `TransferStartPolicy::WaitForEarlier`. Queue
refusal retains the current draft and its choices for retry.

Move removes each source only after that file's successful content proof; earlier
completed moves remain after a later failure or cancel. Failure before that file's
temporary staging leaves its original name. Every pre-deletion failure/cancel after
staging attempts to restore the original name without replacing a new winner.
Successful Move leaves no retained source copy or staging directory. An exceptional
failed restore or cleanup reports its exact temporary location in Jobs. There is no
recovery opt-in, catalogue or journal. See [native transfer guarantees](TRANSFERS.md).

Ownership, ACLs, extended attributes, alternate data streams and provider metadata are
not preserved ([current limits](TRANSFERS.md#current-limits-and-next-stage)).
