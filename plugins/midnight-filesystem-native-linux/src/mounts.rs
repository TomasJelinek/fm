//! Provider-owned mount discovery policy; no host-private helper dependency.
#[cfg(test)]
#[path = "mounts_tests.rs"]
mod drive_list_tests;

pub fn paths(input: &[u8]) -> Vec<Vec<u8>> {
    let mut roots = Vec::new();
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
    roots
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
        assert_eq!(paths(input), [b"/".to_vec(), b"/media/a b\xff".to_vec()]);
    }
}
