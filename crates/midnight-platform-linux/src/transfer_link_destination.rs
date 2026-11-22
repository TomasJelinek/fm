//! What a Copy/Move destination folder's filesystem does with links, for the dialog's
//! default link handling: its `statfs` type, and whether its block device is removable
//! or on a USB bus, read from sysfs. Nothing is written to find out.
use super::*;
use midnight_core::{TransferLinkDestination, TransferLinkStorage};
use std::{os::unix::ffi::OsStrExt, path::Component};

/// Filesystems that cannot store symbolic links, by `statfs` f_type (linux/magic.h):
/// the FAT family (msdos and vfat share `MSDOS_SUPER_MAGIC`) and exFAT.
const NO_LINKS: [(u64, &str); 2] = [(0x4d44, "FAT"), (0x2011_bab0, "exFAT")];

/// Filesystems known to store symbolic links: ext2/3/4, XFS, Btrfs, tmpfs, F2FS, ZFS,
/// bcachefs, NTFS (the kernel's ntfs3 driver) and NFS.
const LINKS: [u64; 9] = [
    0xef53,
    0x5846_5342,
    0x9123_683e,
    0x0102_1994,
    0xf2f5_2010,
    0x2fc1_2fc1,
    0xca45_1a4e,
    0x7366_746e,
    0x6969,
];

/// Network filesystems: SMB (cifs and smb2), NFS and 9p. Their server decides whether a
/// link can be created; only NFS is known to store them.
const NETWORK: [u64; 4] = [0xff53_4d42, 0xfe53_4d42, 0x6969, 0x0102_1997];

/// Most block devices a stack (device mapper, md) is followed through.
const MAX_STACK: usize = 16;

pub(super) fn link_destination(
    path: &Path,
    cancel: &Cancellation,
) -> Result<TransferLinkDestination, FsError> {
    checked(cancel, path)?;
    let path = absolute(path)?;
    let folder = File::from(
        openat(
            CWD,
            &path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|failure| error(&path, failure.into()))?,
    );
    let filesystem = rustix::fs::fstatfs(&folder)
        .map_err(|failure| error(&path, failure.into()))?
        .f_type as u64;
    let device = folder
        .metadata()
        .map_err(|failure| error(&path, failure))?
        .dev();
    let mut facts = classify(filesystem);
    // The mount, as for a drop's device: a rename passes no mount's boundary, and bind
    // mounts of one filesystem share its device number. The device number stands in where
    // the kernel names no mount.
    facts.volume = Some(
        super::native_location::mount(&folder, &path)
            .ok()
            .flatten()
            .unwrap_or(device),
    );
    // The same check that a Move from another drive makes before it removes anything.
    facts.refuses_cross_volume_move = !super::destination::recovery::supported_type(filesystem);
    facts.removable = !facts.network
        && removable_device(
            rustix::fs::major(device),
            rustix::fs::minor(device),
            &|| cancel.is_cancelled(),
        );
    checked(cancel, &path)?;
    Ok(facts)
}

/// The name of the filesystem that holds `folder` when it cannot store symbolic links
/// (FAT, exFAT); None when it can or nothing tells.
pub(super) fn cannot_store_links(folder: &File) -> Option<&'static str> {
    let magic = rustix::fs::fstatfs(folder).ok()?.f_type as u64;
    NO_LINKS
        .iter()
        .find(|(known, _)| *known == magic)
        .map(|(_, name)| *name)
}

/// Link support of the filesystem type `magic`, apart from the device it is on.
fn classify(magic: u64) -> TransferLinkDestination {
    let (storage, filesystem) = match NO_LINKS.iter().find(|(known, _)| *known == magic) {
        Some((_, name)) => (TransferLinkStorage::Unsupported, Some((*name).to_owned())),
        None if LINKS.contains(&magic) => (TransferLinkStorage::Supported, None),
        None => (TransferLinkStorage::Unknown, None),
    };
    TransferLinkDestination {
        storage,
        filesystem,
        network: NETWORK.contains(&magic),
        removable: false,
        volume: None,
        refuses_cross_volume_move: false,
    }
}

/// Whether the block device `major:minor` is removable media or on a USB bus (a stick, a
/// USB disk, a card reader), or an SD card; for a device-mapper or md device, whether
/// one of the devices it is built on is. Anonymous devices (Btrfs subvolumes, tmpfs,
/// network filesystems) have no block device and are not.
fn removable_device(major: u32, minor: u32, cancelled: &dyn Fn() -> bool) -> bool {
    if major == 0 {
        return false;
    }
    Path::new("/sys/dev/block")
        .join(format!("{major}:{minor}"))
        .canonicalize()
        .is_ok_and(|device| removable_node(&device, 0, cancelled))
}

fn removable_node(device: &Path, depth: usize, cancelled: &dyn Fn() -> bool) -> bool {
    if cancelled() || depth > MAX_STACK || !device.starts_with("/sys/devices") {
        return false;
    }
    if on_usb(device) {
        return true;
    }
    // A partition's attributes are those of its disk.
    let disk = if device.join("partition").exists() {
        match device.parent() {
            Some(disk) => disk,
            None => return false,
        }
    } else {
        device
    };
    let attribute = |name: &str| {
        crate::desktop::read_bounded(&disk.join(name), 64).map(|value| value.trim_ascii().to_vec())
    };
    if attribute("removable").as_deref() == Some(b"1")
        || attribute("device/type").as_deref() == Some(b"SD")
    {
        return true;
    }
    let Ok(slaves) = std::fs::read_dir(disk.join("slaves")) else {
        return false;
    };
    slaves.take(MAX_STACK).flatten().any(|slave| {
        slave
            .path()
            .canonicalize()
            .is_ok_and(|slave| removable_node(&slave, depth + 1, cancelled))
    })
}

/// A device below a USB host controller: its sysfs path passes a `usbN` bus.
fn on_usb(device: &Path) -> bool {
    device.components().any(|component| {
        matches!(component, Component::Normal(name)
        if name.as_bytes().strip_prefix(b"usb").is_some_and(|bus| {
            !bus.is_empty() && bus.iter().all(u8::is_ascii_digit)
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat_and_exfat_cannot_store_links_and_are_named() {
        let fat = classify(0x4d44);
        assert_eq!(fat.storage, TransferLinkStorage::Unsupported);
        assert_eq!(fat.filesystem.as_deref(), Some("FAT"));
        assert!(!fat.network);
        let exfat = classify(0x2011_bab0);
        assert_eq!(exfat.storage, TransferLinkStorage::Unsupported);
        assert_eq!(exfat.filesystem.as_deref(), Some("exFAT"));
    }

    #[test]
    fn local_link_filesystems_store_links_and_smb_is_an_unknown_network_location() {
        for magic in [0xef53, 0x5846_5342, 0x9123_683e, 0x0102_1994] {
            assert_eq!(classify(magic).storage, TransferLinkStorage::Supported);
            assert!(!classify(magic).network);
        }
        let nfs = classify(0x6969);
        assert_eq!(nfs.storage, TransferLinkStorage::Supported);
        assert!(nfs.network);
        let smb = classify(0xfe53_4d42);
        assert_eq!(smb.storage, TransferLinkStorage::Unknown);
        assert!(smb.network);
        // FUSE (ntfs-3g, sshfs) and anything unknown: the transfer tries.
        assert_eq!(classify(0x6573_5546).storage, TransferLinkStorage::Unknown);
    }

    #[test]
    fn a_device_below_a_usb_bus_is_removable() {
        let usb = Path::new(
            "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb/sdb1",
        );
        assert!(on_usb(usb));
        let sata = Path::new(
            "/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda/sda1",
        );
        assert!(!on_usb(sata));
        // A name that only starts with "usb" is no bus.
        assert!(!on_usb(Path::new("/sys/devices/virtual/usbmon/block/x")));
        assert!(!on_usb(Path::new("/sys/devices/usb/block/x")));
    }

    #[test]
    fn the_temporary_folder_reports_its_volume_and_cancellation_stops_the_query() {
        let temp = std::env::temp_dir();
        let facts = link_destination(&temp, &Cancellation::default()).unwrap();
        // Its mount, or its device number where the kernel names no mount.
        let mount = super::super::native_location::mount(&File::open(&temp).unwrap(), &temp)
            .unwrap()
            .unwrap_or(std::fs::metadata(&temp).unwrap().dev());
        assert_eq!(facts.volume, Some(mount), "{facts:?}");
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            link_destination(&temp, &cancel).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        let missing = temp.join("midnight-no-such-destination-folder");
        assert_eq!(
            link_destination(&missing, &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::NotFound
        );
    }
}
