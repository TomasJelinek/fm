use crate::volume_details::read_bounded;
use midnight_filesystem_plugin_api::StorageKind;
use std::{
    ffi::OsString,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
};

pub(crate) fn detect(path: &Path, cancelled: &dyn Fn() -> bool) -> StorageKind {
    if cancelled() {
        return StorageKind::Unknown;
    }
    if !path.is_absolute() {
        return StorageKind::Unknown;
    }
    let Some(input) = read_bounded(Path::new("/proc/self/mountinfo"), 1024 * 1024, cancelled)
    else {
        return StorageKind::Unknown;
    };
    // These guards consult local kernel metadata before any target stat/canonicalize.
    let (kind, candidate) = mount_kind(&input, path);
    if kind == StorageKind::Network || candidate.is_none() {
        return kind;
    }
    if cancelled() {
        return StorageKind::Unknown;
    }
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return StorageKind::Unknown;
    };
    let (kind, device) = mount_kind(&input, &canonical);
    if kind == StorageKind::Network {
        return kind;
    }
    let Some((major, minor)) = device else {
        return StorageKind::Unknown;
    };
    if cancelled() {
        return StorageKind::Unknown;
    }
    let Ok(metadata) = std::fs::metadata(&canonical) else {
        return StorageKind::Unknown;
    };
    if libc::major(metadata.dev()) != major || libc::minor(metadata.dev()) != minor {
        return StorageKind::Unknown;
    }
    physical_kind(major, minor, cancelled).unwrap_or(StorageKind::Unknown)
}

fn decoded_path(input: &[u8]) -> Option<PathBuf> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'\\'
            && index + 3 < input.len()
            && (b'0'..=b'3').contains(&input[index + 1])
            && input[index + 2..=index + 3]
                .iter()
                .all(|byte| (b'0'..=b'7').contains(byte))
        {
            output.push(
                (input[index + 1] - b'0') * 64
                    + (input[index + 2] - b'0') * 8
                    + (input[index + 3] - b'0'),
            );
            index += 4;
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    let path = PathBuf::from(OsString::from_vec(output));
    path.is_absolute().then_some(path)
}

fn mount_kind(input: &[u8], path: &Path) -> (StorageKind, Option<(u32, u32)>) {
    if input.len() > 1024 * 1024 {
        return (StorageKind::Unknown, None);
    }
    let mut best = None;
    let mut best_length = 0;
    for (index, line) in input.split(|byte| *byte == b'\n').enumerate() {
        if index >= 8192 {
            return (StorageKind::Unknown, None);
        }
        let Some(separator) = line.windows(3).position(|bytes| bytes == b" - ") else {
            continue;
        };
        let mut fields = line[..separator].split(|byte| *byte == b' ');
        let Some(device) = fields.nth(2) else {
            continue;
        };
        let Some(root) = fields.nth(1).and_then(decoded_path) else {
            continue;
        };
        if !path.starts_with(&root) || root.as_os_str().len() < best_length {
            continue;
        }
        let Some(filesystem) = line[separator + 3..].split(|byte| *byte == b' ').next() else {
            continue;
        };
        best_length = root.as_os_str().len();
        let network = matches!(
            filesystem,
            b"nfs"
                | b"nfs4"
                | b"cifs"
                | b"smb3"
                | b"smbfs"
                | b"9p"
                | b"ceph"
                | b"glusterfs"
                | b"fuse.sshfs"
                | b"fuse.glusterfs"
                | b"fuse.smbnetfs"
                | b"fuse.rclone"
                | b"davfs"
                | b"davfs2"
                | b"afs"
                | b"coda"
        );
        if network {
            best = Some((StorageKind::Network, None));
            continue;
        }
        // Multi-device/stacked/virtual filesystems cannot prove a single physical SSD.
        if !matches!(filesystem, b"ext2" | b"ext3" | b"ext4" | b"xfs" | b"f2fs") {
            best = Some((StorageKind::Unknown, None));
            continue;
        }
        let device = std::str::from_utf8(device)
            .ok()
            .and_then(|device| device.split_once(':'))
            .and_then(|(major, minor)| {
                Some((major.parse::<u32>().ok()?, minor.parse::<u32>().ok()?))
            })
            .filter(|(major, _)| *major != 0);
        best = Some((StorageKind::Unknown, device));
    }
    best.unwrap_or((StorageKind::Unknown, None))
}

fn physical_kind(major: u32, minor: u32, cancelled: &dyn Fn() -> bool) -> Option<StorageKind> {
    let mut disk = Path::new("/sys/dev/block")
        .join(format!("{major}:{minor}"))
        .canonicalize()
        .ok()?;
    if !disk.starts_with("/sys/devices") {
        return None;
    }
    if read_bounded(&disk.join("partition"), 32, cancelled).is_some() {
        disk = disk.parent()?.to_path_buf();
    }
    // Do not traverse device-mapper/RAID dependencies; any stacked leaf is unknown.
    if std::fs::read_dir(disk.join("slaves"))
        .ok()?
        .next()
        .is_some()
    {
        return None;
    }
    let rotational = read_bounded(&disk.join("queue/rotational"), 16, cancelled)?;
    let removable = read_bounded(&disk.join("removable"), 16, cancelled)?;
    let name = disk.file_name()?.as_bytes();
    let (transport, model) = if name.starts_with(b"nvme") {
        let controller = disk.parent()?;
        (
            read_bounded(&controller.join("transport"), 32, cancelled)?,
            read_bounded(&controller.join("model"), 128, cancelled)?,
        )
    } else {
        let vendor = read_bounded(&disk.join("device/vendor"), 64, cancelled)?;
        if vendor.trim_ascii() != b"ATA" {
            return None;
        }
        (
            b"ata".to_vec(),
            read_bounded(&disk.join("device/model"), 128, cancelled)?,
        )
    };
    Some(classify_physical(
        &disk,
        &rotational,
        &removable,
        &transport,
        &model,
    ))
}

fn decimal(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn classify_physical(
    path: &Path,
    rotational: &[u8],
    removable: &[u8],
    transport: &[u8],
    model: &[u8],
) -> StorageKind {
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| {
            if let Component::Normal(name) = part {
                Some(name.as_bytes())
            } else {
                None
            }
        })
        .take(129)
        .collect();
    if parts.len() > 128
        || !path.starts_with("/sys/devices")
        || !parts.iter().any(|part| part.starts_with(b"pci"))
        || parts.iter().any(|part| {
            *part == b"virtual" || part.starts_with(b"usb") || part.starts_with(b"virtio")
        })
        || removable.trim_ascii() != b"0"
    {
        return StorageKind::Unknown;
    }
    let Some(name) = path.file_name().map(|name| name.as_bytes()) else {
        return StorageKind::Unknown;
    };
    let nvme = name
        .strip_prefix(b"nvme")
        .and_then(|tail| {
            tail.iter()
                .position(|byte| *byte == b'n')
                .map(|split| decimal(&tail[..split]) && decimal(&tail[split + 1..]))
        })
        .unwrap_or(false);
    let sata = name
        .strip_prefix(b"sd")
        .is_some_and(|tail| !tail.is_empty() && tail.iter().all(u8::is_ascii_lowercase))
        && parts
            .iter()
            .any(|part| part.strip_prefix(b"ata").is_some_and(decimal));
    if !(nvme && transport.trim_ascii() == b"pcie" || sata && transport.trim_ascii() == b"ata") {
        return StorageKind::Unknown;
    }
    let model = model.trim_ascii();
    if model.is_empty() || model.len() > 128 || !model.is_ascii() {
        return StorageKind::Unknown;
    }
    let lower = model.to_ascii_lowercase();
    if [
        b"virtual".as_slice(),
        b"vmware",
        b"qemu",
        b"vbox",
        b"xen",
        b"microsoft",
        b"msft",
        b"amazon",
        b"google",
        b"elastic",
        b"iscsi",
    ]
    .iter()
    .any(|word| lower.windows(word.len()).any(|part| part == *word))
    {
        return StorageKind::Unknown;
    }
    match rotational.trim_ascii() {
        b"0" => StorageKind::LocalSsd,
        b"1" => StorageKind::Rotational,
        _ => StorageKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

    #[test]
    fn longest_native_mount_match_prioritizes_network_and_component_boundaries() {
        let input = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 0:2 / /mnt/net rw - nfs4 host:/data rw\n3 1 259:1 / /mnt/net/local rw - xfs /dev/nvme0n1 rw\n4 1 0:4 / /mnt/my\\040share\\377 rw - cifs //host/share rw\n";
        assert_eq!(
            mount_kind(input, Path::new("/mnt/net/文档")),
            (StorageKind::Network, None)
        );
        assert_eq!(
            mount_kind(input, Path::new("/mnt/network")),
            (StorageKind::Unknown, Some((8, 1)))
        );
        assert_eq!(
            mount_kind(input, Path::new("/mnt/net/local/👩🏽‍💻")),
            (StorageKind::Unknown, Some((259, 1)))
        );
        let raw = PathBuf::from(OsString::from_vec(b"/mnt/my share\xff/child".to_vec()));
        assert_eq!(mount_kind(input, &raw), (StorageKind::Network, None));
        for fs in [
            "cifs",
            "9p",
            "ceph",
            "fuse.sshfs",
            "nfs",
            "nfs4",
            "glusterfs",
        ] {
            let input = format!("1 0 0:1 / / rw - {fs} server rw\n");
            assert_eq!(
                mount_kind(input.as_bytes(), Path::new("/anything")),
                (StorageKind::Network, None)
            );
        }
        for fs in ["overlay", "btrfs", "tmpfs", "fuse", "zfs"] {
            let input = format!("1 0 8:1 / / rw - {fs} source rw\n");
            assert_eq!(
                mount_kind(input.as_bytes(), Path::new("/anything")),
                (StorageKind::Unknown, None)
            );
        }
    }

    #[test]
    fn only_physical_pcie_nvme_or_ata_with_valid_local_proof_are_fast() {
        let nvme = Path::new("/sys/devices/pci0000:00/0000:00:01.0/nvme/nvme0/nvme0n1");
        let sata = Path::new("/sys/devices/pci0000:00/0000:00:01.0/ata1/host0/target0/block/sda");
        assert_eq!(
            classify_physical(nvme, b"0\n", b"0\n", b"pcie\n", b"Physical SSD"),
            StorageKind::LocalSsd
        );
        assert_eq!(
            classify_physical(sata, b"1\n", b"0\n", b"ata\n", b"Physical HDD"),
            StorageKind::Rotational
        );
        for (path, transport, model) in [
            (
                Path::new("/sys/devices/virtual/block/dm-0"),
                b"pcie".as_slice(),
                b"SSD".as_slice(),
            ),
            (
                Path::new("/sys/devices/pci0000:00/usb1/ata1/block/sda"),
                b"ata",
                b"SSD",
            ),
            (
                Path::new("/sys/devices/pci0000:00/virtio0/block/vda"),
                b"ata",
                b"SSD",
            ),
            (nvme, b"tcp", b"SSD"),
            (nvme, b"pcie", b"QEMU NVMe"),
            (nvme, b"pcie", b"Amazon Elastic Block Store"),
            (nvme, b"pcie", b""),
        ] {
            assert_eq!(
                classify_physical(path, b"0", b"0", transport, model),
                StorageKind::Unknown
            );
        }
        assert_eq!(
            classify_physical(nvme, b"0", b"1", b"pcie", b"SSD"),
            StorageKind::Unknown
        );
        assert_eq!(
            classify_physical(nvme, b"garbage", b"0", b"pcie", b"SSD"),
            StorageKind::Unknown
        );
    }
}
