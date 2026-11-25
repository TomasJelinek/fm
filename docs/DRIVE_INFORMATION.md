# Drive information

**Ctrl+F1** or **Ctrl+L** opens **Drive information** for the volume that holds the
active panel's folder. Inside an archive the dialog names the **Archive container**,
the native archive file, and describes the volume that holds it; it never describes
the archive as a disk. In a Network Neighborhood location it asks you to open a
shared folder first.

## What the dialog shows

- The folder (or archive container), the volume or mount that holds it, the volume
  label when there is one, the filesystem name and a native source or identifier: the
  volume serial number on Windows, the mount source on Linux.
- **Total**, **Used**, **Free on filesystem** and **Available to you**, each in exact
  bytes in your number format and in readable units. Used is total minus the
  filesystem's free space, never total minus what your quota leaves you.
- **Storage**: Local solid-state storage, Rotational storage, Local storage (media
  type unknown), Network filesystem, or Unavailable. Storage is never assumed to be an
  SSD: a disk attached to the machine whose media type cannot be proven (a virtual
  machine's disk, a USB disk) shows "Local storage (media type unknown)", and storage
  that cannot be shown to be local shows "Unavailable".
- **Read-only filesystem**, when the system reports it. It describes the filesystem or
  mount and does not mean that you can write every folder on it.
- **Reported filesystem properties**, only those the system reports: on Windows, for
  example, case handling, Unicode names, access-control lists, compression, sparse
  files, named streams, hard links, reparse points, encryption and quotas; on Linux,
  the mount restrictions (no execution, no set-user-ID, no device files), never
  properties guessed from a filesystem name.
- A field that cannot be read shows "Unavailable", and the reasons are listed at the
  end; a zero reading stays zero. One failed field does not hide the others.

The values are live readings, not a snapshot that later operations rely on. **Copy
details** copies the report; the dialog can be scrolled and resized.

## How the query works

The native filesystem package answers on a backend worker, one query at a time. A
newer request replaces a waiting one and cancels the running one cooperatively; a
system call that is already blocked cannot be interrupted. A reply is shown only
while it still belongs to the same panel, folder, archive and dialog: a late reply
never replaces newer information or reopens a dialog you closed. Paths keep their
exact native form and are never rebuilt from display text.

- **Windows** queries the volume's root with dynamic buffers and without the
  system's removable-media error dialogs. It reads the filesystem name, label, serial
  number and property flags. The total comes from
  [GetDiskSpaceInformationW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskspaceinformationw),
  which is not limited by your quota;
  [GetDiskFreeSpaceExW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskfreespaceexw)
  supplies the filesystem's free bytes and the bytes available to you. Its
  quota-limited "total" is never shown as the volume's capacity: when the real total
  cannot be read, Total and Used stay unavailable.
- **Linux** holds an `O_PATH` descriptor (it opens no file content), reads capacity
  and mount restrictions with `fstatvfs`, and finds the mount by matching the held
  object's `statx` mount ID and device against `/proc/self/mountinfo` (read up to
  1 MiB), so bind mounts and different spellings of a mount path are told apart. The
  label comes from the device's udev record, when there is one. The storage class is
  discarded if the path no longer names the held mount by the time it is known.

## The drive list on Linux

The drive dropdown and the drive bar show `/`, Home and useful mounted storage: local
volumes, removable media, network shares, FUSE mounts of the user and the locations
opened through GVfs. The list comes from `/proc/self/mountinfo` on a backend worker;
the native filesystem package and the application apply the same filter:

- Left out: snap images, namespaces, portals, kernel and tracing filesystems,
  container layers, and mounts under `/dev`, `/proc`, `/sys`, `/boot`, `/efi`,
  `/snap`, `/var/lib/snapd`, `/var/lib/docker`, `/var/lib/containers`, `/tmp`,
  `/var/tmp` and `/run`.
- Kept below `/run`: `/run/media`, and mounts at or below
  `/run/user/<numeric user ID>/gvfs` other than the GVfs FUSE bridge itself.
- The GVfs FUSE bridge is left out; the native filesystem package lists the
  locations it holds instead (see below). The application's own list, which the
  window uses only until the package's list arrives, does not read the bridge.
- A mount of a subfolder of a block device is a second view of a drive already
  listed and is left out, which also hides deliberate bind mounts of local folders;
  Btrfs and bcachefs subvolumes stay listed. A bind mount of a whole filesystem may
  appear twice, because the mount table does not distinguish it.

The filter changes only the choices offered: a folder left out, the bridge included,
can still be typed in the path bar, browsed and described with Ctrl+F1.

### Locations opened through GVfs

GVfs, the desktop's virtual filesystem, mounts each location it opens (from the file
manager, with `gio mount` or from Network Neighborhood) as a folder of one FUSE mount,
the GVfs FUSE bridge at `/run/user/<numeric user ID>/gvfs`. The bridge holds nothing
else and is empty while nothing is mounted, so it is not listed. Each of its folders
is listed as a drive instead, named after its location:

| Location | Folder in the bridge, for example | Drive name |
| --- | --- | --- |
| SMB share | `smb-share:server=nas,share=data` | data on nas |
| SFTP, FTP, WebDAV or another host | `sftp:host=example.com,port=2222` | example.com |
| NFS export | `nfs:host=server,prefix=%2Fexport%2Fdata` | data on server |
| AFP volume | `afp-volume:host=server,volume=Shared` | Shared on server |
| Phone or camera (MTP, PTP) | `mtp:host=Vendor_Phone_1A2B3C` | Vendor Phone 1A2B3C |
| Online account drive | its type, then `host=example.com,user=name` | name@example.com |
| Archive | `archive:host=` and the archive's escaped address | the archive's file name |

A folder name without this form keeps its raw text. A drive name is display text
only; the drive is its folder's path.

Building the list reads only the names of the bridge's folders, at most 256, on the
same backend worker; it never opens or examines a location, whose details come from
its backend and may be slow or offline. The GVfs FUSE daemon answers that read from
memory, without its backends. Should the daemon itself stop answering, for example
when every one of its threads waits for an offline location, the drive list is not
refreshed until it answers again. Free space and the volume name of each location
are read later, like those of every drive, on the separately bounded workers for
drive details; for a location these reads reach its backend. A bridge that cannot be
read, such as another user's, adds no drives, and a filesystem mounted at the bridge's
path makes that path an ordinary drive. GVfs mounts do not change
the mount table, so a location that appears or disappears shows in the list when the
list is read again: at start and every 45 seconds. Drives named after their location
come before drives named by their path.

## Dated records

Salamander's drive information dialog (`CDriveInfo` in `src/dialogs3.cpp`) was a
reference for the kinds of information only; no source or assets were copied. The
dated [verification record](verification/2026-11-08-drive-information.md) lists the
tests and fixtures. The [GVfs drive list record](verification/2026-11-25-gvfs-drive-list.md)
covers the drives of locations opened through GVfs.
