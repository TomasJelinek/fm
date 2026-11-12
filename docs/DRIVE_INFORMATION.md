# Detailed volume information

On Linux, the drive dropdown and drive bar show `/`, the Home shortcut and useful
mounted storage: local volumes, removable media, network shares and user FUSE
mounts. Discovery reads `/proc/self/mountinfo` on backend workers, with the same
filter in the independent native provider and startup desktop locations. Snap
images, namespaces, portals, tracing/kernel filesystems, container layers and
mounts under system/boot/temporary directories are omitted. Runtime exceptions
retain `/run/media` and `/run/user/<numeric UID>/gvfs` mounts.

Block-device subdirectory views are omitted as duplicate navigation locations;
Btrfs and bcachefs subvolumes remain available. This also hides intentional local
directory bind mounts. Whole-filesystem bind aliases may remain because mountinfo
does not distinguish them. Filtering affects navigation choices only: an omitted
path can still be entered directly, browsed and queried for volume information.

Ctrl+F1 and Ctrl+L query the filesystem containing the displayed native folder.
Inside an archive, the dialog explicitly identifies the native **archive container**
and reports its hosting filesystem. It does not describe the archive as a disk.

The dialog shows the actual volume/mount, native label when available, filesystem
name, native source/identifier, total/used/free capacity, bytes available to the
current user, and reliably reported storage classification and filesystem properties.
Capacity has exact locale-formatted bytes and readable units. Used bytes are total
minus filesystem-free bytes, never total minus the user's quota-limited availability.
An unavailable field stays unavailable; a zero reading remains zero. Optional-field
failures preserve the other measured fields. Read-only state describes the filesystem
or mount and does not claim that the caller can write every directory.

Windows queries the resolved Unicode volume root with dynamic buffers, suppresses
native removable-media error dialogs, and reads filesystem name, label, serial and
supported property flags. Actual total capacity comes from
[GetDiskSpaceInformationW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskspaceinformationw),
whose actual allocation units exclude caller quota limitations.
[GetDiskFreeSpaceExW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskfreespaceexw)
supplies filesystem-free bytes and caller-available bytes separately; its quota-limited
"total" is deliberately not displayed as actual volume capacity. If actual total
cannot be queried, total/used remain unavailable. The API fits the existing Windows
10 22H2 target. Existing conservative native device classification is reused.

Linux holds an `O_PATH` descriptor without opening content, obtains capacity and mount
restrictions from `fstatvfs`, and matches the held object's `statx` mount ID and device
against bounded `/proc/self/mountinfo`. This distinguishes bind mounts and identical
mount-path spellings without guessing a mount from path text. Native escaped bytes
remain raw until presentation. An optional label comes from the matching device's
bounded udev database record; absent udev metadata is ordinary unavailability.
Classification uses the existing conservative backend and is discarded if its final
path query no longer matches the held mount. Unknown or virtual storage is never
assumed to be an SSD. Linux capability rows list actual mount restrictions; they do
not infer NTFS-like capabilities from filesystem names.

All native work runs on a dedicated backend actor with one active call and one
coalesced latest request. A newer request cancels prior work cooperatively; an already
blocked OS call cannot be forcibly interrupted. The UI captures a request identity,
pane navigation, raw native path, archive address and notice occurrence. Stale results
cannot replace a newer notice or another location's information. Replies update the
existing notice body without changing its occurrence, so closing the loading dialog
never allows a late reply to reopen it. Closing the application cancels delivery.
The dialog supports scrolling, resizing and Copy details through the existing backend
clipboard action.

Queries are read-only live observations, not filesystem snapshots or authority for
copy/delete decisions. No operation path is truncated or reconstructed from display
text; no `MAX_PATH` buffer, shell command, new dependency or public plugin API is added.
The Salamander `CDriveInfo` implementation in `src/dialogs3.cpp` was inspected for its
information categories only; no source or assets were copied.

Windows and Linux native tests, actor cancellation tests, strict backend checks and
the actual Windows dialog workflow pass. The [verification record](verification/2026-11-08-drive-information.md)
lists commands/scopes, raw/long-path fixtures, layout evidence and remaining limits.
This document makes no macOS, physical-media or untested Windows-version
qualification claim.
