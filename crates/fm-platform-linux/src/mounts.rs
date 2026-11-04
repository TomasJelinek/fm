pub(crate) fn mount_paths(input: &[u8]) -> Vec<Vec<u8>> {
    let mut roots = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        let Some(separator) = line.windows(3).position(|bytes| bytes == b" - ") else {
            continue;
        };
        let Some(mount) = line[..separator].split(|byte| *byte == b' ').nth(4) else {
            continue;
        };
        let Some(filesystem) = line[separator + 3..].split(|byte| *byte == b' ').next() else {
            continue;
        };
        if matches!(
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
        ) {
            continue;
        }
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
        if filesystem == b"tmpfs"
            && [b"/dev".as_slice(), b"/run", b"/sys", b"/proc"]
                .iter()
                .any(|prefix| {
                    decoded == *prefix
                        || (decoded.starts_with(prefix) && decoded.get(prefix.len()) == Some(&b'/'))
                })
        {
            continue;
        }
        if !roots.contains(&decoded) {
            roots.push(decoded);
        }
    }
    roots
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
}
