use crate::desktop::read_bounded;
use midnight_core::StorageKind;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
};

pub(crate) fn detect(path: &Path) -> StorageKind {
    if !path.is_absolute() {
        return StorageKind::Unknown;
    }
    let Some(input) = read_bounded(Path::new("/proc/self/mountinfo"), 1024 * 1024) else {
        return StorageKind::Unknown;
    };
    // These guards consult local kernel metadata before any target stat/canonicalize.
    let (kind, candidate) = mount_kind(&input, path);
    if kind == StorageKind::Network || candidate.is_none() {
        return kind;
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
    let Ok(metadata) = std::fs::metadata(&canonical) else {
        return StorageKind::Unknown;
    };
    if rustix::fs::major(metadata.dev()) != major || rustix::fs::minor(metadata.dev()) != minor {
        return StorageKind::Unknown;
    }
    physical_kind(major, minor).unwrap_or(StorageKind::Unknown)
}

/// Whether `path` is the folder of a GVfs location of a network protocol, such as
/// `smb-share:server=nas,share=data`, directly in GVfs's FUSE bridge. Only the mount
/// table and the folder's name are read; the storage class stays unknown, since the
/// bridge's own filesystem says nothing of the location's storage.
pub(crate) fn gvfs_network_location(path: &Path) -> bool {
    path.is_absolute()
        && read_bounded(Path::new("/proc/self/mountinfo"), 1024 * 1024)
            .is_some_and(|input| gvfs_network_location_in(&input, path))
}

fn gvfs_network_location_in(input: &[u8], path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    // The mount that holds `path`: the longest mount point it is in, the later of
    // two at one path (stacked over the earlier).
    let mut holder: Option<(PathBuf, bool)> = None;
    for line in input.split(|byte| *byte == b'\n').take(8192) {
        let Some(separator) = line.windows(3).position(|bytes| bytes == b" - ") else {
            continue;
        };
        let mut fields = line[..separator].split(|byte| *byte == b' ');
        let (Some(root), Some(Some(mount))) = (fields.nth(3), fields.next().map(decoded_path))
        else {
            continue;
        };
        let filesystem = line[separator + 3..].split(|byte| *byte == b' ').next();
        if path.starts_with(&mount)
            && holder
                .as_ref()
                .is_none_or(|(held, _)| mount.as_os_str().len() >= held.as_os_str().len())
        {
            let bridge = root == b"/" && filesystem == Some(b"fuse.gvfsd-fuse".as_slice());
            holder = Some((mount, bridge));
        }
    }
    let Some((mount, true)) = holder else {
        return false;
    };
    let name = name.as_bytes();
    mount == parent
        && name
            .iter()
            .position(|byte| *byte == b':')
            .is_some_and(|colon| {
                matches!(
                    &name[..colon],
                    b"smb-share"
                        | b"smb-server"
                        | b"sftp"
                        | b"ftp"
                        | b"ftps"
                        | b"ftpis"
                        | b"dav"
                        | b"davs"
                        | b"dav+sd"
                        | b"davs+sd"
                        | b"nfs"
                        | b"afp-volume"
                        | b"afp-server"
                        | b"google-drive"
                        | b"onedrive"
                )
            })
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
        // Single-device block filesystems only: multi-device, stacked, FUSE and virtual
        // filesystems cannot prove which block device, if any, holds their files.
        if !matches!(
            filesystem,
            b"ext2"
                | b"ext3"
                | b"ext4"
                | b"xfs"
                | b"f2fs"
                | b"vfat"
                | b"msdos"
                | b"exfat"
                | b"ntfs3"
        ) {
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

const MAX_STORAGE_DEPTH: usize = 32;
const MAX_STORAGE_NODES: usize = 128;

#[derive(Clone)]
enum BlockNode {
    Partition(PathBuf),
    Backing(Vec<PathBuf>),
    Physical(StorageKind),
}

struct StorageGraph<'a> {
    cancelled: &'a dyn Fn() -> bool,
    probe: &'a mut dyn FnMut(&Path) -> Option<BlockNode>,
    active: HashSet<PathBuf>,
    completed: HashMap<PathBuf, StorageKind>,
    visited: usize,
}

impl StorageGraph<'_> {
    fn visit(&mut self, disk: &Path, depth: usize) -> Option<StorageKind> {
        if (self.cancelled)() || depth > MAX_STORAGE_DEPTH {
            return None;
        }
        if let Some(kind) = self.completed.get(disk) {
            return Some(*kind);
        }
        if self.visited >= MAX_STORAGE_NODES || !self.active.insert(disk.to_owned()) {
            return None;
        }
        self.visited += 1;
        let kind = match (self.probe)(disk)? {
            BlockNode::Partition(parent) => self.visit(&parent, depth + 1)?,
            BlockNode::Backing(children)
                if !children.is_empty() && children.len() <= MAX_STORAGE_NODES =>
            {
                // Every leaf must be local. A rotational leaf makes the stack rotational;
                // otherwise one leaf of unknown media leaves the stack's media unknown.
                let mut kind = StorageKind::LocalSsd;
                for child in children {
                    kind = match (kind, self.visit(&child, depth + 1)?) {
                        (_, StorageKind::Network | StorageKind::Unknown) => return None,
                        (StorageKind::Rotational, _) | (_, StorageKind::Rotational) => {
                            StorageKind::Rotational
                        }
                        (StorageKind::Local, _) | (_, StorageKind::Local) => StorageKind::Local,
                        _ => StorageKind::LocalSsd,
                    };
                }
                kind
            }
            BlockNode::Physical(
                kind @ (StorageKind::LocalSsd | StorageKind::Rotational | StorageKind::Local),
            ) => kind,
            _ => return None,
        };
        if (self.cancelled)() {
            return None;
        }
        self.active.remove(disk);
        self.completed.insert(disk.to_owned(), kind);
        Some(kind)
    }
}

fn classify_graph(
    root: PathBuf,
    cancelled: &dyn Fn() -> bool,
    probe: &mut dyn FnMut(&Path) -> Option<BlockNode>,
) -> StorageKind {
    StorageGraph {
        cancelled,
        probe,
        active: HashSet::new(),
        completed: HashMap::new(),
        visited: 0,
    }
    .visit(&root, 0)
    .unwrap_or(StorageKind::Unknown)
}

/// Read at most one excess entry to reject a directory larger than the graph budget.
fn collect_backing(
    mut entries: impl Iterator<Item = io::Result<PathBuf>>,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<PathBuf>> {
    let mut backing = Vec::new();
    loop {
        if cancelled() {
            return None;
        }
        let Some(entry) = entries.next() else {
            return Some(backing);
        };
        if backing.len() >= MAX_STORAGE_NODES {
            return None;
        }
        let path = entry.ok()?;
        if !path.starts_with("/sys/devices") {
            return None;
        }
        backing.push(path);
    }
}

fn physical_kind(major: u32, minor: u32) -> Option<StorageKind> {
    let disk = Path::new("/sys/dev/block")
        .join(format!("{major}:{minor}"))
        .canonicalize()
        .ok()?;
    Some(classify_graph(disk, &|| false, &mut |disk| {
        probe_block(disk, &|| false)
    }))
}

fn probe_block(disk: &Path, cancelled: &dyn Fn() -> bool) -> Option<BlockNode> {
    if cancelled() || !disk.starts_with("/sys/devices") {
        return None;
    }
    let partition = disk.join("partition");
    if partition.try_exists().ok()? {
        let number = read_bounded(&partition, 32)?;
        if !decimal(number.trim_ascii()) {
            return None;
        }
        return Some(BlockNode::Partition(disk.parent()?.to_owned()));
    }
    let backing = collect_backing(
        std::fs::read_dir(disk.join("slaves"))
            .ok()?
            .map(|entry| entry.and_then(|entry| entry.path().canonicalize())),
        cancelled,
    )?;
    if !backing.is_empty() {
        // Follow only recognized kernel block stacks. Their physical leaves must
        // independently prove local storage; a stack's own rotational flag is insufficient.
        let name = disk.file_name()?.as_bytes();
        if !name.strip_prefix(b"dm-").is_some_and(decimal)
            && !name.strip_prefix(b"md").is_some_and(decimal)
        {
            return None;
        }
        return Some(BlockNode::Backing(backing));
    }
    let rotational = read_bounded(&disk.join("queue/rotational"), 16)?;
    let removable = read_bounded(&disk.join("removable"), 16)?;
    let name = disk.file_name()?.as_bytes();
    let (transport, model) = if name.starts_with(b"nvme") {
        let controller = disk.parent()?;
        (
            read_bounded(&controller.join("transport"), 32)?,
            read_bounded(&controller.join("model"), 128)?,
        )
    } else {
        // libata names its disks' vendor "ATA". Other disks (a virtual machine's, USB,
        // SCSI, MMC or virtio disks) may name no vendor or model at all.
        let ata = read_bounded(&disk.join("device/vendor"), 64)
            .is_some_and(|vendor| vendor.trim_ascii() == b"ATA");
        (
            if ata { b"ata".to_vec() } else { Vec::new() },
            read_bounded(&disk.join("device/model"), 128).unwrap_or_default(),
        )
    };
    Some(BlockNode::Physical(classify_physical(
        disk,
        &rotational,
        &removable,
        &transport,
        &model,
    )))
}

fn decimal(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

/// A leaf block device by its canonical sysfs path and metadata: an SSD or a rotational
/// disk only when a physical PCIe NVMe or libata disk proves it; `Local` for any other
/// disk attached to this machine (a virtual or cloud machine's disk, a USB, removable,
/// SCSI, MMC or virtio disk); `Unknown` when the disk may be elsewhere.
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
    if parts.len() > 128 || !attached_here(path, &parts) {
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
    let transport = transport.trim_ascii();
    let model = model.trim_ascii();
    let lower = model.to_ascii_lowercase();
    let names = |words: &[&[u8]]| {
        words
            .iter()
            .any(|word| lower.windows(word.len()).any(|part| part == *word))
    };
    // An NVMe namespace reached over TCP, RDMA, Fibre Channel or a loop target, or a
    // disk that names iSCSI, is a target that may be on another machine.
    if (nvme && transport != b"pcie") || names(&[b"iscsi"]) {
        return StorageKind::Unknown;
    }
    let sata = name
        .strip_prefix(b"sd")
        .is_some_and(|tail| !tail.is_empty() && tail.iter().all(u8::is_ascii_lowercase))
        && parts
            .iter()
            .any(|part| part.strip_prefix(b"ata").is_some_and(decimal));
    let proven = parts.iter().any(|part| part.starts_with(b"pci"))
        && !parts
            .iter()
            .any(|part| part.starts_with(b"usb") || part.starts_with(b"virtio"))
        && removable.trim_ascii() == b"0"
        && (nvme || (sata && transport == b"ata"))
        && !model.is_empty()
        && model.len() <= 128
        && model.is_ascii()
        && !names(&[
            b"virtual",
            b"vmware",
            b"qemu",
            b"vbox",
            b"xen",
            b"microsoft",
            b"msft",
            b"amazon",
            b"google",
            b"elastic",
        ]);
    match (proven, rotational.trim_ascii()) {
        (true, b"0") => StorageKind::LocalSsd,
        (true, b"1") => StorageKind::Rotational,
        _ => StorageKind::Local,
    }
}

/// A leaf's device path belongs to a disk attached to this machine: below
/// `/sys/devices`, but not a virtual block device (loop, nbd, rbd, zram, an NVMe
/// fabrics or multipath namespace) and not behind an iSCSI session or a Fibre Channel
/// remote port.
fn attached_here(path: &Path, parts: &[&[u8]]) -> bool {
    path.starts_with("/sys/devices")
        && !parts.iter().any(|part| {
            *part == b"virtual"
                || part.strip_prefix(b"session").is_some_and(decimal)
                || part.starts_with(b"rport-")
        })
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
        for fs in ["overlay", "btrfs", "tmpfs", "fuse", "zfs", "fuseblk"] {
            let input = format!("1 0 8:1 / / rw - {fs} source rw\n");
            assert_eq!(
                mount_kind(input.as_bytes(), Path::new("/anything")),
                (StorageKind::Unknown, None)
            );
        }
        // Single-device block filesystems name the block device that holds them.
        for fs in ["ext4", "xfs", "vfat", "exfat", "ntfs3"] {
            let input = format!("1 0 8:17 / /media/stick rw - {fs} /dev/sdb1 rw\n");
            assert_eq!(
                mount_kind(input.as_bytes(), Path::new("/media/stick/photos")),
                (StorageKind::Unknown, Some((8, 17)))
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
        // Attached to this machine, but not a physical disk that proves its media.
        for (path, transport, model) in [
            (
                Path::new("/sys/devices/pci0000:00/usb1/ata1/block/sda"),
                b"ata".as_slice(),
                b"SSD".as_slice(),
            ),
            (
                Path::new("/sys/devices/pci0000:00/virtio0/block/vda"),
                b"ata",
                b"SSD",
            ),
            (nvme, b"pcie", b"QEMU NVMe"),
            (nvme, b"pcie", b"Amazon Elastic Block Store"),
            (nvme, b"pcie", b""),
        ] {
            assert_eq!(
                classify_physical(path, b"0", b"0", transport, model),
                StorageKind::Local,
                "{path:?} {model:?}"
            );
        }
        assert_eq!(
            classify_physical(nvme, b"0", b"1", b"pcie", b"SSD"),
            StorageKind::Local
        );
        assert_eq!(
            classify_physical(nvme, b"garbage", b"0", b"pcie", b"SSD"),
            StorageKind::Local
        );
        // Virtual block devices and remote transports prove nothing local.
        for (path, transport, model) in [
            (
                Path::new("/sys/devices/virtual/block/dm-0"),
                b"pcie".as_slice(),
                b"SSD".as_slice(),
            ),
            (Path::new("/sys/devices/virtual/block/nbd0"), b"", b""),
            (nvme, b"tcp", b"SSD"),
            (nvme, b"rdma", b"SSD"),
            (nvme, b"loop", b"SSD"),
            (
                Path::new("/sys/devices/virtual/nvme-fabrics/ctl/nvme1/nvme1n1"),
                b"pcie",
                b"SSD",
            ),
            (
                Path::new("/sys/devices/platform/host3/session1/target3:0:0/3:0:0:0/block/sdb"),
                b"",
                b"VIRTUAL-DISK",
            ),
            (
                Path::new(
                    "/sys/devices/pci0000:00/0000:00:03.0/host5/rport-5:0-0/target5:0:0/5:0:0:0/block/sdc",
                ),
                b"",
                b"SAN LUN",
            ),
            (
                Path::new(
                    "/sys/devices/pci0000:00/0000:00:02.0/host2/target2:0:0/2:0:0:0/block/sdd",
                ),
                b"",
                b"iSCSI Disk",
            ),
            (Path::new("/proc/self/block/sda"), b"ata", b"SSD"),
        ] {
            assert_eq!(
                classify_physical(path, b"0", b"0", transport, model),
                StorageKind::Unknown,
                "{path:?} {model:?}"
            );
        }
    }

    /// THUMB-UNKNOWN-STORAGE: the disks of WSL 2 and other virtual and cloud machines,
    /// and removable disks, are final `Local` storage, never `Unknown`.
    #[test]
    fn virtual_machine_and_removable_disks_are_local_of_unknown_media() {
        for (path, rotational, removable, transport, model) in [
            // WSL 2 and Hyper-V: a SCSI disk on the VMBus, vendor "Msft".
            (
                "/sys/devices/LNXSYSTM:00/LNXSYBUS:00/ACPI0004:00/VMBUS:00/\
                 fd1d2cbd-ce7c-535c-966b-eb5f811c95f0/host0/target0:0:0/0:0:0:0/block/sdc",
                b"1".as_slice(),
                b"0".as_slice(),
                b"".as_slice(),
                b"Virtual Disk".as_slice(),
            ),
            // VMware's SCSI disk and VirtualBox's SATA disk.
            (
                "/sys/devices/pci0000:00/0000:00:10.0/host2/target2:0:0/2:0:0:0/block/sda",
                b"1",
                b"0",
                b"",
                b"Virtual disk",
            ),
            (
                "/sys/devices/pci0000:00/0000:00:0d.0/ata3/host2/target2:0:0/2:0:0:0/block/sda",
                b"0",
                b"0",
                b"ata",
                b"VBOX HARDDISK",
            ),
            // Xen, MMC and a USB stick.
            ("/sys/devices/vbd-51712/block/xvda", b"0", b"0", b"", b""),
            (
                "/sys/devices/platform/soc/fe340000.mmc/mmc_host/mmc0/mmc0:aaaa/block/mmcblk0",
                b"0",
                b"0",
                b"",
                b"",
            ),
            (
                "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb",
                b"1",
                b"1",
                b"",
                b"Flash Disk",
            ),
        ] {
            assert_eq!(
                classify_physical(Path::new(path), rotational, removable, transport, model),
                StorageKind::Local,
                "{path}"
            );
        }
    }

    #[test]
    fn native_temporary_mount_hint_is_read_only_and_proc_is_not_proven_ssd() {
        let directory = tempfile::tempdir().unwrap();
        let _ = detect(directory.path());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        assert_ne!(detect(Path::new("/proc")), StorageKind::LocalSsd);
    }
}

#[cfg(test)]
#[path = "storage_graph_tests.rs"]
mod graph_tests;

#[cfg(test)]
#[path = "storage_kind_gvfs_tests.rs"]
mod gvfs_tests;
