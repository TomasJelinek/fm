# Delete confirmation summary

Delete confirmation should remain easy to review with a large selection. A single
file shows its original filename, logical byte size, and modified date/time. Multiple
targets show file count and total file bytes, plus folder count when applicable.
Folder-only selections show the folder count. The dialog never lists the selected
paths and never starts a recursive size scan to open the confirmation.

## Implementation

The existing bounded mutation preparation worker already pins every selected native
identity and reads its metadata. Accumulate file bytes in that pass, across the
complete prepared selection, independently of the preview sample's 32-entry bound.
Only regular-file logical lengths contribute; directories and directory links never
contribute folder contents. Missing regular-file sizes, unclassified non-directory
entries, and arithmetic overflow produce an unavailable total rather than a partial
or wrapped number. Keep the aggregate explicitly labelled `Files size`.

The UI formats counts and exact byte lengths through the immutable `NumberLocale`
snapshot. A single file uses the prepared item's `localized_modified`, with the
existing pure, explicitly UTC fallback when only a timestamp is available. Display
names retain their Unicode spelling; display strings never become operation paths.
The already captured source directory remains context for the dialog.

The two existing Trash/permanent prompts, cancel-first focus, explicit confirmation
token checks, asynchronous execution, native identity revalidation, and navigation
generation checks stay in force. No platform API, dependency, or license changes
are required.

## Approaches and reference

Summing the preview sample would miss targets beyond its bound. Reusing the browsing
selection total would describe older listing metadata and can include unknown
directory sizes. The selected implementation aggregates the metadata captured by
the existing preparation pass, adding constant work per already prepared target.

Salamander's `src/fileswn8.cpp`, around its selection subject construction and delete
confirmation, distinguishes a single named entry from plural file/folder counts.
This is behavioral inspiration only; no reference source or assets are copied.

## Verification

Exercise the real dialog presentation for multiple files, mixed entries, one file,
and folder-only selections. Check Unicode, native locale grouping/date strings,
unknown metadata, counts beyond 32-bit range, and overflow-safe rendering. Backend
tests verify that the aggregate covers targets beyond the preview sample, uses
prepared metadata, excludes directories, rejects a false exact total on missing
sizes/overflow, and performs no deletion or recursive discovery during preparation.
Existing mutation control and operation tests retain coverage for the confirmation
mode, cancellation, stale tokens, and native identity/generation handling.
