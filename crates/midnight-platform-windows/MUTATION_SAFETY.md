# Windows filesystem mutations

The Windows provider runs blocking filesystem and Shell calls on engine workers.
Tests create fresh fixtures below `_build/tmp`. No test enumerates or empties the
user's Recycle Bin. The three explicitly ignored Shell tests require a separate
native run outside the filesystem sandbox; the successful recycle test restores
only the exact `IShellItem` returned by its own operation.

The junction fixture verifies that its target is readable before recycling, remains
intact while the junction is absent, and is still accessible after restoring only
that junction. PowerShell's fixture creator requires ordinary target spelling;
passing an extended prefix creates an invalid junction on the tested host.

## Identity and paths

Preparing rename/delete retains two handles: the parent directory and the
selected entry opened without following its final reparse point. The recorded
identity combines the volume identifier and the full 128-bit file identifier.
Holding the original handle prevents identifier reuse while confirmation is
pending. Before mutation, the provider reopens and compares the visible parent
and selected entry. Native failure, replacement, cancellation and exhausted OS
resources are errors.

Rename uses `NtSetInformationFile(FileRenameInformation)` with a basename,
null root directory and replacement disabled. That operation renames inside the
source handle's own directory. Mutation handles deny concurrent delete/rename
access. Case-only rename and destination collision are tested on the host's NTFS
volume. Destination entries are never removed first, and replacement is never
enabled.

Mutation handles request `DELETE`, read-attributes and synchronization access.
File-content read and directory-list access are unnecessary for rename and leaf
deletion. ACL fixtures explicitly deny those rights and verify both operations.

Permanent deletion enumerates by directory handle and opens each child relative
to that handle with `FILE_OPEN_REPARSE_POINT`. Reparse points, including directory
junctions, are leaves. Ordinary directories acquire a separate enumeration handle
relative to their pinned parent while retaining the original mutation guard. Its
identity and reparse state are revalidated before enumeration. An iterative stack
retains two directory handles per active depth, plus the current child and the
prepared handles; it does not open every
descendant simultaneously or impose a depth/path-length ceiling. Memory includes
the pending names for each active directory. Cancellation is checked during
enumeration and before each removal. Progress is reported after each removed
entry. Concurrent additions can cause an explicit nonempty-directory failure;
contents are not a snapshot captured at confirmation time.

`FileDispositionInfoEx` with delete and POSIX semantics removes the namespace
entry while identity handles remain open. Filesystem/OS versions without the
required identity or disposition support return their native error. Explicitly
confirmed permanent deletion adds `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`
so a DOS Read-only flag does not reject an otherwise authorized deletion. The
flag does not bypass DELETE access checks, alter ACLs or clear the attribute on
surviving hard links. Copy/Move cleanup retains the original disposition without
this override. A failure or cancellation does not restore entries already removed.
See the [read-only deletion verification](../../docs/verification/2026-11-09-readonly-delete.md).

F7 retains one base-directory handle and revalidates its visible identity. It
validates the entire explicit request before creation, then uses handle-relative
`NtCreateFile` for normal components. Absolute disk/UNC targets start at their own
root; root-relative targets use the captured panel root. Same-drive relative
targets use the captured base; another drive's relative current directory is
ambiguous and requires an absolute path. Relative requests containing `..` pin
the captured base's canonical ancestor chain and verify the base identity before
creation. Each entered component is traversed before a later parent step, so a
link followed by `..` cannot be silently elided. Existing intermediate directories are
accepted, reparse intermediates are rejected, and the final component uses
`FILE_CREATE` so an existing entry is an error. Existing intermediate handles
remain open along the descent. Native errors can leave already-created
intermediates; there is no automatic rollback.

Empty-file creation shares the prepared base and identity revalidation with F7.
It accepts exactly one validated basename and uses handle-relative `NtCreateFile`
with `FILE_CREATE`, `FILE_NON_DIRECTORY_FILE` and `FILE_OPEN_REPARSE_POINT`. The
new file starts at zero bytes; existing files, directories and links are errors
and remain unchanged. Cancellation is checked before the synchronous create call.
Concurrent requests for the same name are resolved exclusively by the filesystem;
the fixture race verifies exactly one succeeds. No editor is launched by this
backend operation.

## Recycle Bin

Trash uses the dedicated `ITransferSource::RecycleItem` operation. It never calls
`RemoveItem`, `IFileOperation::DeleteItem`, or permanent deletion as a fallback.
The Shell returns the item at its recycle location. Errors such as unavailable
recycling, forced permanent-delete configuration, excessive item size, or an
unsupported recycle path length are surfaced to the user.

The tested Windows Shell rejects a source path longer than its recycle limit
with `COPYENGINE_E_RECYCLE_PATH_TOO_LONG`. The provider returns `Unsupported` and
the fixture remains intact. Rename, permanent deletion and recursive creation
are separately tested beyond `MAX_PATH`; the application does not impose that
limit. The Shell parser accepts the ordinary drive/UNC spelling, so its input
removes only the extended path prefix while preserving UTF-16 code units. Names
whose interpretation could change through Shell normalization are rejected at
preparation.

The Shell does not expose a recycle call bound to an already-open filesystem
handle. Identity is revalidated immediately before `RecycleItem`, but a
concurrent namespace replacement inside that native call remains a race. The
provider does not claim atomic identity validation for recycling. An in-flight
native call is not forcibly interrupted; cancellation is checked before it and
between selected root items by the engine.

## Native references

- [File identity](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
- [Handle-relative access and reparse behavior](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile)
- [Native rename contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)
- [Explicit recycle operation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itransfersource-recycleitem)
- [Distinct recycle and permanent-removal methods](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-itransfersource)

Behavioral references inspected: Salamander `src/fileswn5.cpp:1946` for recursive
creation and `:2108`/`:2329` for rename, and `src/fileswn8.cpp:56` for recycling.
Empty-file behavior was compared with `src/fileswn5.cpp:1474` (`EditNewFile`) and
the Shift+F4 dispatch in `src/salamand.rc:76` / `src/mainwnd3.cpp:3579`.
Its ANSI buffers, path caps, and destination-delete-before-rename fallback are
not used.
