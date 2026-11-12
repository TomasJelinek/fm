//! Provider-owned mount discovery policy; no host-private helper dependency.
pub fn paths(input: &[u8]) -> Vec<Vec<u8>> {
    let mut roots = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        let Some(separator) = line.windows(3).position(|part| part == b" - ") else {
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
        let mut decoded = Vec::new();
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
            roots.push(decoded)
        }
    }
    roots
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
