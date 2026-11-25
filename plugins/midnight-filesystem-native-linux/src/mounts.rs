//! Provider-owned mount discovery policy; no host-private helper dependency.
#[cfg(test)]
#[path = "mounts_tests.rs"]
mod drive_list_tests;

/// The mount table as the drive list offers it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Mounts {
    /// Mount points offered as drives, in mount-table order.
    pub roots: Vec<Vec<u8>>,
    /// GVfs FUSE bridges, whose folders are offered in their place.
    pub gvfs: Vec<Vec<u8>>,
}

pub fn paths(input: &[u8]) -> Mounts {
    let mut roots = Vec::new();
    let mut gvfs = Vec::new();
    let mut seen = std::collections::HashSet::<Vec<u8>>::new();
    for line in input.split(|byte| *byte == b'\n') {
        let Some(separator) = line.windows(3).position(|bytes| bytes == b" - ") else {
            continue;
        };
        let mut fields = line[..separator].split(|byte| *byte == b' ');
        let Some(root) = fields.nth(3) else {
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
        if !seen.contains(decoded.as_slice()) {
            seen.insert(decoded.clone());
            roots.push(decoded);
        }
    }
    // Another filesystem mounted at the bridge's path, over or under it, makes that
    // path an ordinary drive; its folders are not GVfs locations.
    gvfs.retain(|bridge| !seen.contains(bridge));
    Mounts { roots, gvfs }
}

/// The drive list from the mount table, as raw roots with display labels: `/` and the
/// mount points, labeled by path, then the locations in the GVfs bridges, labeled by
/// name.
pub fn drives(mounts: Mounts, cancelled: &dyn Fn() -> bool) -> Vec<(Vec<u8>, String)> {
    let Mounts { mut roots, gvfs } = mounts;
    if !roots.iter().any(|root| root == b"/") {
        roots.insert(0, b"/".to_vec());
    }
    let locations = crate::gvfs::locations(&gvfs, &roots, cancelled);
    roots
        .into_iter()
        .map(|root| {
            let label = String::from_utf8_lossy(&root).into_owned();
            (root, label)
        })
        .chain(
            locations
                .into_iter()
                .map(|location| (location.root, location.label)),
        )
        .collect()
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
