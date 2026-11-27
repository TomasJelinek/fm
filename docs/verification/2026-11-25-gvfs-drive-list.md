# GVfs locations in the Linux drive list — 25 November 2026

The Linux drive list no longer offers the GVfs FUSE bridge,
`/run/user/<numeric user ID>/gvfs`, as one empty read-only drive. The native
filesystem package lists each location mounted through GVfs, one folder of the
bridge, as a drive of its own, named after the location
([drive list](../drive-information.md#locations-opened-through-gvfs)). The
application's Linux platform adapter, whose list feeds the desktop context and is
shown only until the package's list arrives, leaves the bridge out without reading
it.

## Folder names

The naming rules come from the GVfs 1.60.0 sources. `g_mount_spec_to_string`
(`common/gmountspec.c`) writes the type, a colon and comma-separated `key=value` items,
keys in sorted order and values percent-escaped; `daemon/mount.c` and
`client/gvfsfusedaemon.c` use that string as the folder name without escaping it
again, and `gvfsd-fuse` answers the listing of its root from its in-memory mount list,
with no entry attributes. Mounts that are not user-visible get no folder: with the
Trash location mounted, the bridge stayed empty.

## A real GVfs mount

On an Ubuntu desktop with GVfs 1.60.0, a test build of the native package listed the
volumes through its exported interface (an in-process client of the plugin ABI, so the
reply passed the wire checks and had unique identities). The probe test was temporary
and is not part of the change.

| State | Drive list |
| --- | --- |
| Nothing mounted through GVfs | `/` only; the bridge is not listed |
| A local zip archive named `a, b – ž.zip` mounted with `gio mount archive://…` | `/` and a drive named `a, b – ž.zip`, the name GVfs itself gives the mount, rooted at the archive's folder in the bridge |
| After `gio mount -u` | `/` only |

Under `strace`, the only calls on the bridge were an `openat` of the bridge, an `fstat`
of the bridge itself and `getdents64`; no entry of the bridge was examined or opened.

## Targeted validation

Linux, the project's Docker build image, a private target directory, `--locked
--offline`.

| Command | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `cargo test --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml --lib -- mounts:: gvfs::` | 22 | 0 | 0 |
| `cargo test --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml` (unit and integration tests) | 109 | 0 | 1 |
| `cargo test -p midnight-platform-linux --lib -- mounts::` | 14 | 0 | 0 |
| `cargo test -p midnight-platform-linux --lib` | 448 | 37 | 36 |

The 37 failures are transfer fixtures that need a local lookup domain the Docker
temporary directory does not offer (33 report "fixture requires qualified local byte
lookup"). The same 37 tests fail on the base commit 6f37db8c in the same setup, which
passes 446; the 2 more passing tests are this change's new tests.

The package's new tests cover folder names of SMB, SFTP, FTP, WebDAV, NFS, AFP, MTP,
PTP, online-account, archive and audio-disc locations, names that keep their raw text
(missing items, incomplete or invalid escapes, text that is not UTF-8, control
characters), the bridge's detection in the mount table with and without GVfs mounts
(other filesystems at, over, under or below its path stay ordinary drives and their
folders are not read), the drive entries and their labels, the 256-location limit
across bridges, missing and unreadable bridges, locations already listed as mounts,
and cancellation. Listing a bridge whose entries cannot be examined (no search
permission) still lists them; with a `metadata()` filter added to the code, that test
failed. The platform adapter's new tests cover leaving the bridge out and keeping the
other mounts at or below its path.

`cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` passed for both
packages, as did `git diff --check` and `bash scripts/check-source-size.sh`. Not run:
the workspace and UI suites and the protected-installation host test of the native
package, which is ignored by default. No real SMB, SFTP or MTP location was mounted;
their labels are covered by the folder-name tests above.

## Review and known limits

An independent read-only review found no defect in parsing, identities or bounds. It
raised two limits, kept as they are here:

- Building the drive list waits for the GVfs FUSE daemon to answer the listing of its
  root. The daemon answers from memory, but if it stops answering (for example, every
  one of its threads waits for an offline location), the drive list is not refreshed
  until it answers again. A time limit inside the native package would need a thread
  that outlives the call; the host unloads the package once no call is active, so that
  needs a decision on the plugin interface first.
- The per-drive details (free space, volume name) of a GVfs location are read every
  45 seconds, as for every drive, and these reads reach the location's backend. Two
  offline locations can hold both details workers, delaying the free space of other
  drives, as other offline network mounts already can.

## Follow-ups

Three follow-ups of the first change, after the drive names of
[the Linux drive names record](2026-11-25-linux-drive-names.md) had landed:

- **The drive list right after opening a share.** When a Network Neighborhood
  activation hands a share's native folder to a panel, the panel asks for the drive
  list again (`Command::ListVolumes`, coalesced by the engine), so the new GVfs
  location shows without waiting for the 45-second refresh. A handoff for an older
  navigation asks for nothing.
- **Network icon.** A drive root that is a folder directly in the GVfs FUSE bridge
  (the `fuse.gvfsd-fuse` mount holding it, with root `/`, read from the mount table)
  and names a network protocol (SMB, SFTP, FTP, WebDAV, NFS, AFP or an online account)
  gets the network icon. Phones, cameras, archives, discs, folders inside a location,
  stacked mounts and bind views of the bridge keep the drive icon. The storage class is
  unchanged, so the panel keeps the location's own drive selected and Drive
  information reports its storage as Unavailable.
- **Locations of one name.** GVfs locations that would share a name add the user,
  domain and port from their folder names (`example.com (alice, port 2222)`);
  locations whose names hold nothing more keep the name, and the drive toolbar then
  takes folders from their paths.

| Command | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `cargo test -p midnight-ui --lib -- presenter:: volume_presentation` | 1283 | 0 | 4 |
| `cargo test -p midnight-platform-linux --lib` | 463 | 37 | 36 |
| `cargo test --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml` | 124 | 0 | 1 |

Of the six new tests, the presenter test, the network-protocol icon test and the
naming test failed before their changes; the other three guard what must stay as it
was (other locations and paths keep the drive icon, names without details keep them). The 37 platform failures are the transfer fixtures described
above. `cargo fmt -- --check` and Clippy with `-D warnings` passed for the three
packages (`midnight-ui` for its library and tests).
