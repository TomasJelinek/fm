# Windows filesystem mutations

The Windows provider runs blocking filesystem and Shell calls on engine workers.
Mutation tests create fresh fixtures below `_build/tmp` (pause and discovery tests
use `_build/qa`). Junctions and non-surrogate reparse points are created natively
with `FSCTL_SET_REPARSE_POINT` (`src/test_fixtures.rs`), without a shell. No test
enumerates or empties the user's Recycle Bin. The three explicitly ignored Shell
tests require a separate native run outside the filesystem sandbox; the successful
recycle test restores only the exact `IShellItem` returned by its own operation.

The ignored Recycle Bin junction fixture verifies that its target is readable before
recycling, remains intact while the junction is absent, and is still accessible after
restoring only that junction. It still uses the PowerShell fixture creator, which
requires ordinary target spelling; an extended prefix creates an invalid junction.

## Identity and paths

Preparing rename/delete retains two handles: the parent directory and the
selected entry opened without following its final reparse point. The recorded
identity combines the volume identifier and the full 128-bit file identifier
(filesystems without one: see [FAT32 and exFAT](#fat32-and-exfat)).
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

Windows refuses to rename a directory with `ERROR_ACCESS_DENIED` while any entry
below it is open, whatever that handle's sharing, and antivirus, indexing and sync
programs open files for a moment after they are written (Microsoft Defender: about
10-50 ms per new file on the test host). Rename and the metadata editor's case change
therefore repeat a refused directory rename through the same handle after 10 ms, then
after twice as long each time, for about one second in all, and then report the
refusal. They retry only a directory, and only while reopening the parent for
`FILE_ADD_SUBDIRECTORY` succeeds: NTFS refuses a caller without that right with the
same error, and opening the source already checked `DELETE`, so a refusal by the
parent directory's DACL is reported at once. Any other refusal of a directory rename
(a program, including this application's watch of a subfolder, keeps an entry below
open, or a filter driver refuses the rename) is repeated and reported after about one
second, once per refused folder. Another handle never blocks renaming a file this
handle may delete. During the wait the source handle still denies delete sharing, so
neither the directory nor its ancestors can be renamed, and the rename stays
no-replace. Cancellation ends the wait, and a paused job waits paused. Copy and Move
do not wait (see [transfers](../../docs/developer/TRANSFERS.md)): Copy publishes only
staged files, and a refused Move stops with its Retry/Skip/Cancel decision. Recycling
does not wait either (see [Recycle Bin](#recycle-bin)).

Mutation handles request `DELETE`, read-attributes and synchronization access.
File-content read and directory-list access are unnecessary for rename and leaf
deletion. ACL fixtures explicitly deny those rights and verify both operations.

Permanent deletion enumerates by directory handle and opens each child relative
to that handle with `FILE_OPEN_REPARSE_POINT`. Name-surrogate reparse points
(symbolic links, directory junctions, volume mount points) are leaves: the link
itself is removed, never its target. Other reparse tags (Cloud Files/OneDrive
placeholders, deduplicated or WOF-compressed entries) classify by their directory
attribute, so such directories are recursed like ordinary ones. Ordinary directories
acquire a separate enumeration handle relative to their pinned parent while
retaining the original mutation guard. Its identity and link state are revalidated
before enumeration. A child that another process removed after the directory
snapshot is already in the requested end state and is skipped. An iterative stack
retains two directory handles per active depth, plus the current child and the
prepared handles; it does not open every
descendant simultaneously or impose a depth/path-length ceiling. Memory includes
the pending names for each active directory. Cancellation is checked during
enumeration and before each removal. Progress is reported after each removed
entry. Concurrent additions can cause an explicit nonempty-directory failure;
contents are not a snapshot captured at confirmation time. The Delete job offers each
step that fails (opening, listing or removing an entry, removing a folder after its
entries) for a decision (`delete_with_decisions`): Retry repeats it (an entry is
opened again by its name, a folder's removal reuses its guard), Skip keeps that entry
and the folders that contain it, and Cancel ends the deletion. Without decisions the
first failure ends it.

`FileDispositionInfoEx` with delete and POSIX semantics removes the namespace
entry while identity handles remain open. Explicitly
confirmed permanent deletion adds `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`
so a DOS Read-only flag does not reject an otherwise authorized deletion. The
flag does not bypass DELETE access checks, alter ACLs or clear the attribute on
surviving hard links. Copy/Move cleanup retains the original disposition without
this override and never uses the classic fallback below. A failure or cancellation
does not restore entries already removed.
See the [read-only deletion verification](../../docs/verification/2026-11-09-readonly-delete.md).

Only Find content reads and Copy/Move source reads open file data, and only for a
filter-backed file (Cloud Files placeholder, deduplicated file) do they reopen the
pinned object through the filter stack, so it may hydrate; mutations never do. That
reopen evaluates the reparse point the object carries at that moment. Should another
process with write access to that file replace its filter's reparse point with a
symbolic link between classification and the reopen, the open follows the link before
the caller's identity check rejects the result. The reopen therefore requests identification-level impersonation only: a
named pipe reached that way can identify the process but never impersonate it.
Copy/Move read with write sharing denied; Find, like any reader, shares write and
delete access.

### FAT32 and exFAT

FAT32, exFAT and some redirectors implement neither POSIX disposition nor the
`FileAttributeTagInformation` and `FileIdInformation` classes (the FAT driver answers
them with `STATUS_INVALID_PARAMETER`). Classification then uses the basic
attributes: these filesystems have no reparse points, and a reparse point whose tag
cannot be read is treated as a link. Confirmed permanent deletion falls back to the
classic handle-bound `FileDispositionInfo` on the same pinned no-follow handle. The
name then disappears only when the last handle to the entry closes: a descendant's
at once (its handles close right after its disposition), the selected entry's when
the job releases its prepared handles at its end. Another process's open handle
leaves an entry delete-pending, and its parent then fails as nonempty. The classic
class cannot ignore the DOS Read-only attribute: a confirmed deletion clears it on the
entry itself first, through a second handle to the same object (an empty name relative
to the pinned handle, never a path), and puts it back when the disposition still
fails. A read-only file with other names (hard links), which may lie outside the
confirmed tree, keeps the attribute and its deletion is reported.

Deletion, deletion discovery, Rename, F7, empty-file creation, metadata editing and
Find then identify entries by the 32-bit volume serial number and the 64-bit file
index. FAT derives that index from the position of the entry's directory record:
renaming an entry to a longer name, moving it or defragmenting the volume changes it,
and a new entry can reuse a freed value. On such volumes these operations compare
identities only before they rename an entry. (Metadata editing compares again after
its case-only rename solely to change compression, which these filesystems do not
offer, so that edit is refused there first.) Copy and Move compare identities across
their own stage publication, Move
capture and namespace pins, so they require the 128-bit `FileIdInfo` and refuse such
volumes as sources and destinations with an actionable `Unsupported` error.

This FAT behaviour follows the driver's documented responses; it has not yet run on
real FAT32/exFAT media (the test host has none and cannot create one without
administrator rights). The opt-in
`tests/mutations.rs::removable_media_creation_discovery_and_permanent_deletion`
qualifies it: set `MIDNIGHT_TEST_FAT_DIRECTORY` to a writable directory on such media
and run it with `--ignored`. On any other filesystem it checks the ordinary path.

F7 retains one base-directory handle and revalidates its visible identity. It
validates the entire explicit request before creation, then uses handle-relative
`NtCreateFile` for normal components. Absolute disk/UNC targets start at their own
root; root-relative targets use the captured panel root. Same-drive relative
targets use the captured base; another drive's relative current directory is
ambiguous and requires an absolute path. Relative requests containing `..` pin
the captured base's canonical ancestor chain and verify the base identity before
creation. Each entered component is traversed before a later parent step, so a
link followed by `..` cannot be silently elided. Existing intermediate directories are
accepted (including cloud placeholder directories), link intermediates (name-surrogate
reparse points) are rejected, and the final component uses
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
unsupported recycle path length are surfaced to the user as failures of the job
that keep the item.

Before a Trash delete is confirmed, `trash_support` (`mutation_trash_support.rs`)
tells the engine whether the Recycle Bin takes items from each selected item's
folder, so that the user confirms a permanent delete instead of a Trash delete
that would fail. It only reads: the `NoRecycleFiles` policy (HKLM before HKCU),
the drive type of the folder's volume (found from the retained parent handle with
`GetFinalPathNameByHandleW(VOLUME_NAME_GUID)`, so a volume mounted in a folder or
reached through `subst` counts as itself; a UNC path is a network share without
touching the network) and, on a fixed disk, the volume's `NukeOnDelete` under
`HKCU\...\Explorer\BitBucket\Volume\{volume GUID}`, or the older all-drives
`NukeOnDelete` for a volume without its own. Network, removable, CD/DVD and RAM
drives have no Recycle Bin, except inside a redirected known folder. What it cannot
tell is "available": `RecycleItem` still decides. An answer is reused for two
seconds per volume (path prefix and serial number). The Recycle Bin and its
contents are never opened.

Windows also refuses to recycle a folder while a program keeps an entry below it open,
this application included (its watch of a subfolder that a panel shows or that is
expanded, or a file that one of its viewers shows): the Shell then fails with
`COPYENGINE_E_ACCESS_DENIED_SRC` (0x80270021). That code and
`COPYENGINE_E_SHARING_VIOLATION_SRC` (0x80270027, the Shell's sharing-violation code for
the item) are reported as `ResourceBusy` with the code and the path, saying that the item
or something inside it is open in a program, which can be this application (a panel, an
expanded folder or a viewer), and to retry when it is no longer open; the message for
0x80270021, which the Shell also returns when access is denied, names that cause too.
Unlike Rename, recycling does not wait such a refusal out, so moving a folder to the
Recycle Bin right after its files were written, while a virus scanner may still be
reading them, can fail; the folder then stays in place.

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
