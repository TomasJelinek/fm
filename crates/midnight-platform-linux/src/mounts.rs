#[cfg(test)]
#[path = "mounts_tests.rs"]
mod drive_list_tests;

pub(crate) fn mount_paths(input: &[u8]) -> Vec<Vec<u8>> {
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
    fn mountinfo_decodes_octal_escapes_and_preserves_raw_bytes() {
        let input = b"40 1 8:1 / /media/my\\040disk rw - ext4 /dev/sda rw\n41 1 8:2 / /mnt/a\\011b\\012c\\134d\\377 rw - ext4 /dev/sdb rw\n";
        assert_eq!(
            mount_paths(input),
            vec![b"/media/my disk".to_vec(), b"/mnt/a\tb\nc\\d\xff".to_vec()]
        );
    }

    #[test]
    fn mountinfo_filters_internal_filesystems_and_duplicate_mounts() {
        let input = b"1 0 0:1 / / rw - overlay overlay rw\n2 1 0:2 / /proc rw - proc proc rw\n3 1 0:3 / /sys rw - sysfs sysfs rw\n4 1 0:4 / /dev rw - devtmpfs devtmpfs rw\n5 1 0:5 / /run rw - tmpfs tmpfs rw\n6 1 0:6 / /mnt/data rw - xfs /dev/sdb rw\n7 1 0:6 / /mnt/data rw - xfs /dev/sdb rw\n8 1 0:7 / /sys/fs/cgroup rw - cgroup2 cgroup rw\nmalformed line\n";
        assert_eq!(
            mount_paths(input),
            vec![b"/".to_vec(), b"/mnt/data".to_vec()]
        );
    }

    #[test]
    fn mountinfo_deduplicates_decoded_raw_paths_in_first_occurrence_order() {
        let input = b"1 0 8:1 / /mnt/z rw - ext4 x rw\n2 0 8:1 / /mnt/raw\\377 rw - ext4 x rw\n3 0 8:1 / /mnt/raw\\376 rw - ext4 x rw\n4 0 8:1 / /mnt/\\172 rw - ext4 x rw\n5 0 8:1 / /mnt/a\\040disk rw - ext4 x rw\n6 0 8:1 / /mnt/raw\\377 rw - ext4 x rw\n7 0 8:1 / /mnt/\\141\\040disk rw - ext4 x rw\n";
        assert_eq!(
            mount_paths(input),
            [
                b"/mnt/z".to_vec(),
                b"/mnt/raw\xff".to_vec(),
                b"/mnt/raw\xfe".to_vec(),
                b"/mnt/a disk".to_vec(),
            ]
        );
    }

    #[test]
    fn mountinfo_filters_internal_components_without_hiding_prefix_siblings() {
        let input = b"1 0 0:1 / /run rw - tmpfs x rw\n2 0 0:1 / /run/sub rw - tmpfs x rw\n3 0 0:1 / /runner rw - tmpfs x rw\n4 0 0:1 / /mnt/tmp rw - tmpfs x rw\n5 0 8:1 / /run/user-disk rw - ext4 x rw\n";
        assert_eq!(
            mount_paths(input),
            [b"/runner".to_vec(), b"/mnt/tmp".to_vec()]
        );
    }

    #[test]
    fn large_repeated_mountinfo_emits_each_decoded_root_once() {
        let input = b"1 0 8:1 / /mnt/a rw - ext4 x rw\n2 0 8:1 / /mnt/\\141 rw - ext4 x rw\n"
            .repeat(25_000);
        assert_eq!(mount_paths(&input), [b"/mnt/a".to_vec()]);
    }
}
