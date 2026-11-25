//! Provider-owned mount discovery policy; no host-private helper dependency.
use crate::drive_names;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
#[path = "mounts_tests.rs"]
mod drive_list_tests;

/// One drive of the list: its identity, raw path and name for the drive toolbar.
pub struct Named {
    pub id: String,
    pub path: Vec<u8>,
    pub name: String,
}

/// The mount table as the drive list offers it.
#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Mounts {
    /// Mount points offered as drives, in mount-table order.
    pub roots: Vec<Vec<u8>>,
    /// GVfs FUSE bridges, whose folders are offered in their place.
    pub gvfs: Vec<Vec<u8>>,
}

/// [`Mounts`] with the device of each mount point, for its filesystem label.
struct Table {
    /// Mount points offered as drives, in mount-table order.
    mounts: Vec<MountPoint>,
    /// GVfs FUSE bridges, whose folders are offered in their place.
    gvfs: Vec<Vec<u8>>,
}

/// One mount the drive list keeps: its decoded path and its device (`major:minor`).
struct MountPoint {
    path: Vec<u8>,
    device: (u32, u32),
}

/// The drive list from `input` (`/proc/self/mountinfo`): `/` and the mount points, then
/// the locations in the GVfs bridges, each named for the drive toolbar
/// ([`drive_names`]): a mount point by the filesystem label `device_label` finds for its
/// device (asked once per device) or else its folder, a GVfs location by the location.
pub fn volumes(
    input: &[u8],
    device_label: &mut dyn FnMut(u32, u32) -> Option<Vec<u8>>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Named> {
    named(table(input), device_label, cancelled)
}

fn named(
    table: Table,
    device_label: &mut dyn FnMut(u32, u32) -> Option<Vec<u8>>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Named> {
    let Table { mut mounts, gvfs } = table;
    if !mounts.iter().any(|mount| mount.path == b"/") {
        mounts.insert(
            0,
            MountPoint {
                path: b"/".to_vec(),
                device: (0, 0),
            },
        );
    }
    let mut known = HashMap::new();
    let mut labels: Vec<Option<Vec<u8>>> = mounts
        .iter()
        .map(|mount| {
            // Anonymous devices (major 0: tmpfs, FUSE, network) have no udev record.
            let (major, minor) = mount.device;
            (major != 0)
                .then(|| {
                    known
                        .entry(mount.device)
                        .or_insert_with(|| device_label(major, minor))
                        .clone()
                })
                .flatten()
        })
        .collect();
    let mut paths: Vec<Vec<u8>> = mounts.into_iter().map(|mount| mount.path).collect();
    for location in crate::gvfs::locations(&gvfs, &paths, cancelled) {
        paths.push(location.root);
        labels.push(Some(location.label.into_bytes()));
    }
    let names = drive_names::names(
        &paths
            .iter()
            .zip(&labels)
            .map(|(path, label)| drive_names::Mount {
                path,
                label: label.as_deref(),
            })
            .collect::<Vec<_>>(),
    );
    paths
        .into_iter()
        .zip(names)
        .map(|(path, name)| Named {
            // Hex identity avoids collisions between distinct non-UTF-8 mount paths.
            id: format!(
                "mount:{}",
                path.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            path,
            name,
        })
        .collect()
}

/// The mount points that the drive list offers, and the GVfs bridges.
#[cfg(test)]
pub fn paths(input: &[u8]) -> Mounts {
    let Table { mounts, gvfs } = table(input);
    Mounts {
        roots: mounts.into_iter().map(|mount| mount.path).collect(),
        gvfs,
    }
}

/// The mount points of `input` that the drive list keeps, each path once, where it
/// first appears; its device is that of the last mount there, the one on top. GVfs
/// bridges are set apart.
fn table(input: &[u8]) -> Table {
    let mut roots: Vec<MountPoint> = Vec::new();
    let mut gvfs = Vec::new();
    let mut seen = HashMap::<Vec<u8>, usize>::new();
    for line in input.split(|byte| *byte == b'\n') {
        let Some(separator) = line.windows(3).position(|bytes| bytes == b" - ") else {
            continue;
        };
        let mut fields = line[..separator].split(|byte| *byte == b' ');
        let Some(device) = fields.nth(2) else {
            continue;
        };
        let Some(root) = fields.next() else {
            continue;
        };
        let Some(mount) = fields.next() else {
            continue;
        };
        let mut fields = line[separator + 3..].split(|byte| *byte == b' ');
        let Some(filesystem) = fields.next() else {
            continue;
        };
        let Some(source) = fields.next() else {
            continue;
        };
        let mut decoded = Vec::with_capacity(mount.len());
        let mut index = 0;
        while index < mount.len() {
            if mount[index] == b'\\'
                && index + 3 < mount.len()
                && (b'0'..=b'3').contains(&mount[index + 1])
                && (b'0'..=b'7').contains(&mount[index + 2])
                && (b'0'..=b'7').contains(&mount[index + 3])
            {
                decoded.push(
                    (mount[index + 1] - b'0') * 64
                        + (mount[index + 2] - b'0') * 8
                        + (mount[index + 3] - b'0'),
                );
                index += 4;
            } else {
                decoded.push(mount[index]);
                index += 1;
            }
        }
        if !decoded.starts_with(b"/") {
            continue;
        }
        // GVfs serves every location it mounts as a folder of one FUSE mount that
        // holds nothing else, so the drive list offers those folders instead.
        if root == b"/" && filesystem == b"fuse.gvfsd-fuse" && gvfs_bridge(&decoded) {
            if !gvfs.contains(&decoded) {
                gvfs.push(decoded);
            }
            continue;
        }
        // This is a navigation list, not an inventory of the mount namespace.
        // Keep the root even in containers/live systems. A block-device subtree
        // is a bind view rather than another drive; native subvolumes are useful.
        if decoded != b"/"
            && (internal_filesystem(filesystem)
                || internal_path(&decoded)
                || (root != b"/"
                    && source.starts_with(b"/dev/")
                    && !matches!(filesystem, b"btrfs" | b"bcachefs")))
        {
            continue;
        }
        let device = device_number(device);
        match seen.get(&decoded) {
            Some(&index) => roots[index].device = device,
            None => {
                seen.insert(decoded.clone(), roots.len());
                roots.push(MountPoint {
                    path: decoded,
                    device,
                });
            }
        }
    }
    // Another filesystem mounted at the bridge's path, over or under it, makes that
    // path an ordinary drive; its folders are not GVfs locations.
    let listed: HashSet<&[u8]> = roots.iter().map(|mount| mount.path.as_slice()).collect();
    gvfs.retain(|bridge| !listed.contains(bridge.as_slice()));
    Table {
        mounts: roots,
        gvfs,
    }
}

/// `major:minor` as numbers; `(0, 0)`, an anonymous device, when malformed.
fn device_number(field: &[u8]) -> (u32, u32) {
    std::str::from_utf8(field)
        .ok()
        .and_then(|field| field.split_once(':'))
        .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)))
        .unwrap_or((0, 0))
}

/// The path of GVfs's FUSE bridge: `/run/user/<numeric user ID>/gvfs`.
fn gvfs_bridge(path: &[u8]) -> bool {
    path.strip_prefix(b"/run/user/")
        .and_then(|rest| rest.strip_suffix(b"/gvfs"))
        .is_some_and(|uid| !uid.is_empty() && uid.iter().all(u8::is_ascii_digit))
}

fn internal_filesystem(filesystem: &[u8]) -> bool {
    matches!(
        filesystem,
        b"proc"
            | b"sysfs"
            | b"devtmpfs"
            | b"devpts"
            | b"cgroup"
            | b"cgroup2"
            | b"pstore"
            | b"securityfs"
            | b"debugfs"
            | b"tracefs"
            | b"configfs"
            | b"hugetlbfs"
            | b"mqueue"
            | b"rpc_pipefs"
            | b"binfmt_misc"
            | b"fusectl"
            | b"autofs"
            | b"efivarfs"
            | b"bpf"
            | b"squashfs"
            | b"nsfs"
            | b"fuse.portal"
            | b"fuse.snapfuse"
            | b"overlay"
            | b"aufs"
            | b"fuse.overlayfs"
    )
}

fn within(path: &[u8], parent: &[u8]) -> bool {
    path == parent || (path.starts_with(parent) && path.get(parent.len()) == Some(&b'/'))
}

fn internal_path(path: &[u8]) -> bool {
    if [
        b"/dev".as_slice(),
        b"/proc",
        b"/sys",
        b"/boot",
        b"/efi",
        b"/snap",
        b"/var/lib/snapd",
        b"/var/lib/docker",
        b"/var/lib/containers",
        b"/tmp",
        b"/var/tmp",
    ]
    .iter()
    .any(|parent| within(path, parent))
    {
        return true;
    }
    within(path, b"/run") && !within(path, b"/run/media") && !gvfs_path(path)
}

fn gvfs_path(path: &[u8]) -> bool {
    let Some(relative) = path.strip_prefix(b"/run/user/") else {
        return false;
    };
    let mut parts = relative.splitn(2, |byte| *byte == b'/');
    matches!((parts.next(), parts.next()), (Some(uid), Some(rest))
        if !uid.is_empty() && uid.iter().all(u8::is_ascii_digit) && within(rest, b"gvfs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mount_names_preserve_raw_bytes_and_exclude_internal_filesystems() {
        let input=b"1 0 0:1 / / rw - overlay overlay rw\n2 1 0:2 / /proc rw - proc proc rw\n3 1 0:3 / /media/a\\040b\\377 rw - ext4 /dev/x rw\n";
        assert_eq!(
            paths(input).roots,
            [b"/".to_vec(), b"/media/a b\xff".to_vec()]
        );
    }
}
