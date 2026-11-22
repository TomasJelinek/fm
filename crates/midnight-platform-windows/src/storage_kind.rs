use midnight_core::StorageKind;
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Storage::FileSystem::{
        BusType1394, BusTypeAta, BusTypeAtapi, BusTypeFibre, BusTypeMmc, BusTypeNvme, BusTypeRAID,
        BusTypeSCM, BusTypeSas, BusTypeSata, BusTypeScsi, BusTypeSd, BusTypeSsa, BusTypeUfs,
        BusTypeUsb, BusTypeiScsi, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::{
            DEVICE_SEEK_PENALTY_DESCRIPTOR, DISK_EXTENT, IOCTL_STORAGE_QUERY_PROPERTY,
            PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
            StorageDeviceProperty, StorageDeviceSeekPenaltyProperty, VOLUME_DISK_EXTENTS,
        },
        WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE},
    },
};

fn unc(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)))
}

fn wide(path: &Path) -> Option<Vec<u16>> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(wide)
}

fn drive_root(path: &Path) -> Option<PathBuf> {
    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) {
        return None;
    }
    let mut root = prefix.as_os_str().to_os_string();
    root.push("\\");
    Some(root.into())
}

fn drive_type(root: &Path) -> Option<u32> {
    let wide = wide(root)?;
    // SAFETY: live NUL-terminated Unicode root; this only queries the drive category.
    Some(unsafe { GetDriveTypeW(wide.as_ptr()) })
}

pub(crate) fn detect(path: &Path) -> StorageKind {
    // Recognized remote paths must not be canonicalized/stat'ed first.
    if unc(path) {
        return StorageKind::Network;
    }
    let Some(root) = drive_root(path) else {
        return StorageKind::Unknown;
    };
    match drive_type(&root) {
        Some(DRIVE_REMOTE) => return StorageKind::Network,
        // A removable drive (a USB stick, a card reader) is classified by its disk too.
        Some(DRIVE_FIXED | DRIVE_REMOVABLE) => {}
        _ => return StorageKind::Unknown,
    }
    // An empty card reader must not raise a system media-error dialog.
    let Ok(_mode) = crate::volume_details::ErrorMode::new() else {
        return StorageKind::Unknown;
    };
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return StorageKind::Unknown;
    };
    if unc(&canonical) {
        return StorageKind::Network;
    }
    detect_local(&canonical).unwrap_or(StorageKind::Unknown)
}

/// Whether `path` is on a network location: a UNC path or a mapped network drive.
pub(crate) fn network(path: &Path) -> bool {
    unc(path)
        || drive_root(path).is_some_and(|root| drive_type(&root) == Some(DRIVE_REMOTE))
        || std::fs::canonicalize(path).is_ok_and(|canonical| unc(&canonical))
}

/// Whether `path` is on removable media or on a disk attached by a removable bus (USB,
/// an SD or MMC card reader), as its drive type and its disk's storage descriptor report
/// them. Nothing is read from the disk itself; what cannot be told is not removable.
pub(crate) fn removable(path: &Path) -> bool {
    if unc(path) {
        return false;
    }
    std::fs::canonicalize(path)
        .ok()
        .and_then(|canonical| removable_local(&canonical))
        .unwrap_or(false)
}

fn removable_local(path: &Path) -> Option<bool> {
    let volume = volume(path)?;
    match volume.drive_type {
        DRIVE_REMOVABLE => return Some(true),
        DRIVE_FIXED => {}
        _ => return Some(false),
    }
    removable_descriptor(&property(&volume.disk()?, StorageDeviceProperty)?)
}

/// A disk with removable media, or on a USB, SD or MMC bus.
fn removable_descriptor(device: &[u8]) -> Option<bool> {
    let device = validated(device, size_of::<STORAGE_DEVICE_DESCRIPTOR>())?;
    let bus = u32_at(device, offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType))? as i32;
    Some(
        device[offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia)] != 0
            || [BusTypeUsb, BusTypeSd, BusTypeMmc].contains(&bus),
    )
}

fn query_handle(path: &Path) -> Option<File> {
    // Zero requested access permits property queries when allowed by the user's ACL;
    // no raw sectors are read and no elevation/retry with greater access is attempted.
    OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .ok()
}

/// The volume that holds a canonical path: its root and that root's drive type.
struct Volume {
    /// The NUL-terminated mount-point root, as `GetVolumePathNameW` returned it.
    root: Vec<u16>,
    drive_type: u32,
}

fn volume(path: &Path) -> Option<Volume> {
    let input = wide(path)?;
    // A mount-point prefix cannot exceed its canonical input path. The extra space
    // guarantees the trailing separator and terminator without a MAX_PATH buffer.
    let mut root = vec![0u16; input.len().checked_add(2)?];
    let capacity = u32::try_from(root.len()).ok()?;
    // SAFETY: input is terminated and output is writable for its stated capacity.
    if unsafe { GetVolumePathNameW(input.as_ptr(), root.as_mut_ptr(), capacity) } == 0 {
        return None;
    }
    let length = root.iter().position(|unit| *unit == 0)?;
    root.truncate(length + 1);
    // SAFETY: the returned root is terminated and validated by the OS.
    let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
    Some(Volume { root, drive_type })
}

impl Volume {
    /// A property-query handle to the one disk that holds this volume; none for a volume
    /// that spans disks or whose disk cannot be queried.
    fn disk(&self) -> Option<File> {
        // A volume GUID identifier has a documented fixed format, unrelated to path length.
        let mut guid = [0u16; 64];
        // SAFETY: valid root and bounded output for the volume GUID identifier.
        if unsafe {
            GetVolumeNameForVolumeMountPointW(
                self.root.as_ptr(),
                guid.as_mut_ptr(),
                guid.len() as u32,
            )
        } == 0
        {
            return None;
        }
        let length = guid.iter().position(|unit| *unit == 0)?;
        if length == 0 || guid[length - 1] != b'\\' as u16 {
            return None;
        }
        let volume = query_handle(Path::new(&OsString::from_wide(&guid[..length - 1])))?;
        let extents = ioctl(
            &volume,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            &[],
            size_of::<VOLUME_DISK_EXTENTS>(),
        )?;
        if extents.len() < size_of::<VOLUME_DISK_EXTENTS>() || u32_at(&extents, 0)? != 1 {
            return None;
        }
        let disk_number = u32_at(
            &extents,
            offset_of!(VOLUME_DISK_EXTENTS, Extents) + offset_of!(DISK_EXTENT, DiskNumber),
        )?;
        query_handle(Path::new(&format!(r"\\.\PhysicalDrive{disk_number}")))
    }
}

fn detect_local(path: &Path) -> Option<StorageKind> {
    let volume = volume(path)?;
    match volume.drive_type {
        DRIVE_REMOTE => return Some(StorageKind::Network),
        DRIVE_FIXED | DRIVE_REMOVABLE => {}
        _ => return None,
    }
    let disk = volume.disk()?;
    let device = property(&disk, StorageDeviceProperty)?;
    // A disk whose driver does not answer the seek-penalty query keeps its bus's
    // classification: local, of unknown media type.
    let seek = property(&disk, StorageDeviceSeekPenaltyProperty);
    Some(classify_descriptor(&device, seek.as_deref()))
}

fn ioctl(file: &File, code: u32, input: &[u8], capacity: usize) -> Option<Vec<u8>> {
    if capacity > 4096 {
        return None;
    }
    let mut output = vec![0u8; capacity];
    let mut returned = 0u32;
    // SAFETY: synchronous, read-only query codes; live handle and initialized buffers
    // with exact lengths. The driver cannot retain these pointers after completion.
    let success = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            if input.is_empty() {
                ptr::null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if success == 0 || returned as usize > output.len() {
        return None;
    }
    output.truncate(returned as usize);
    Some(output)
}

fn property(file: &File, property: i32) -> Option<Vec<u8>> {
    // Byte encoding also initializes SDK-structure padding; no driver-provided bool is
    // ever transmuted into a Rust bool with potentially invalid bit representation.
    let mut query = [0u8; size_of::<STORAGE_PROPERTY_QUERY>()];
    query[offset_of!(STORAGE_PROPERTY_QUERY, PropertyId)..][..4]
        .copy_from_slice(&property.to_le_bytes());
    query[offset_of!(STORAGE_PROPERTY_QUERY, QueryType)..][..4]
        .copy_from_slice(&PropertyStandardQuery.to_le_bytes());
    ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, &query, 4096)
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn validated(bytes: &[u8], minimum: usize) -> Option<&[u8]> {
    let version = u32_at(bytes, 0)? as usize;
    let size = u32_at(bytes, 4)? as usize;
    if version < minimum || version > size || size < minimum || size > bytes.len() || size > 4096 {
        return None;
    }
    Some(&bytes[..size])
}

fn identity(bytes: &[u8], field: usize) -> Option<&[u8]> {
    let offset = u32_at(bytes, field)? as usize;
    if offset < offset_of!(STORAGE_DEVICE_DESCRIPTOR, RawDeviceProperties) {
        return None;
    }
    let tail = bytes.get(offset..)?;
    let length = tail.iter().position(|byte| *byte == 0)?;
    if length == 0 || length > 128 || !tail[..length].iter().all(|byte| (32..=126).contains(byte)) {
        return None;
    }
    Some(&tail[..length])
}

fn names(identity: &[u8], words: &[&[u8]]) -> bool {
    let lower = identity.to_ascii_lowercase();
    words
        .iter()
        .any(|word| lower.windows(word.len()).any(|part| part == *word))
}

fn virtual_identity(identity: &[u8]) -> bool {
    names(
        identity,
        &[
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
        ],
    )
}

/// Buses of disks attached to this machine, whatever their media. Not iSCSI and Fibre
/// Channel (network storage), file-backed virtual disks (a mounted VHD may live on a
/// share), Storage Spaces (Storage Spaces Direct pools span machines), software-defined
/// virtual buses or buses this build does not know.
fn attached_here(bus: i32) -> bool {
    [
        BusTypeScsi,
        BusTypeAtapi,
        BusTypeAta,
        BusType1394,
        BusTypeSsa,
        BusTypeUsb,
        BusTypeRAID,
        BusTypeSas,
        BusTypeSata,
        BusTypeSd,
        BusTypeMmc,
        BusTypeNvme,
        BusTypeSCM,
        BusTypeUfs,
    ]
    .contains(&bus)
}

/// An SSD or a rotational disk only when a nonremovable, non-virtual ATA, SATA or NVMe
/// disk and its seek-penalty answer prove it; `Local` for any other disk on a bus of
/// this machine (a virtual or cloud machine's disk, a USB, SD, SCSI, SAS or RAID disk)
/// and for a disk whose driver gives no seek-penalty answer (`seek` is `None`);
/// `Unknown` for malformed driver data and disks that may be elsewhere.
fn classify_descriptor(device: &[u8], seek: Option<&[u8]>) -> StorageKind {
    let Some(device) = validated(device, size_of::<STORAGE_DEVICE_DESCRIPTOR>()) else {
        return StorageKind::Unknown;
    };
    let Some(bus) =
        u32_at(device, offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType)).map(|value| value as i32)
    else {
        return StorageKind::Unknown;
    };
    if bus == BusTypeiScsi || bus == BusTypeFibre {
        return StorageKind::Network;
    }
    let removable = match device[offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia)] {
        0 => false,
        1 => true,
        _ => return StorageKind::Unknown,
    };
    let model = identity(
        device,
        offset_of!(STORAGE_DEVICE_DESCRIPTOR, ProductIdOffset),
    );
    let vendor_offset = u32_at(
        device,
        offset_of!(STORAGE_DEVICE_DESCRIPTOR, VendorIdOffset),
    )
    .unwrap_or(0);
    let vendor = identity(
        device,
        offset_of!(STORAGE_DEVICE_DESCRIPTOR, VendorIdOffset),
    );
    // A disk that names iSCSI is a target that may be on another machine.
    if !attached_here(bus)
        || [model, vendor]
            .into_iter()
            .flatten()
            .any(|identity| names(identity, &[b"iscsi"]))
    {
        return StorageKind::Unknown;
    }
    let proven = [BusTypeAta, BusTypeSata, BusTypeNvme].contains(&bus)
        && !removable
        && model.is_some_and(|model| !virtual_identity(model))
        && (vendor_offset == 0 || vendor.is_some_and(|vendor| !virtual_identity(vendor)));
    let Some(seek) = seek.filter(|_| proven) else {
        return StorageKind::Local;
    };
    let Some(seek) = validated(seek, size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()) else {
        return StorageKind::Unknown;
    };
    match seek[offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty)] {
        0 => StorageKind::LocalSsd,
        1 => StorageKind::Rotational,
        _ => StorageKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};
    use windows_sys::Win32::{
        Storage::FileSystem::{
            BusTypeAta, BusTypeFileBackedVirtual, BusTypeMmc, BusTypeNvme, BusTypeSas, BusTypeSata,
            BusTypeScsi, BusTypeSd, BusTypeSpaces, BusTypeUnknown, BusTypeUsb, BusTypeVirtual,
            BusTypeiScsi,
        },
        System::Ioctl::{DEVICE_SEEK_PENALTY_DESCRIPTOR, STORAGE_DEVICE_DESCRIPTOR},
    };

    fn descriptor(bus: i32, model: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; size_of::<STORAGE_DEVICE_DESCRIPTOR>()];
        let header = bytes.len() as u32;
        bytes[0..4].copy_from_slice(&header.to_le_bytes());
        bytes[offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType)..][..4]
            .copy_from_slice(&bus.to_le_bytes());
        bytes[offset_of!(STORAGE_DEVICE_DESCRIPTOR, ProductIdOffset)..][..4]
            .copy_from_slice(&header.to_le_bytes());
        bytes.extend_from_slice(model);
        bytes.push(0);
        let length = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes
    }
    fn seek(penalty: u8) -> Vec<u8> {
        let mut bytes = vec![0; size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()];
        let length = bytes.len() as u32;
        bytes[0..4].copy_from_slice(&length.to_le_bytes());
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes[offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty)] = penalty;
        bytes
    }

    #[test]
    fn only_positive_local_nonremovable_nonvirtual_no_seek_disks_allow_parallelism() {
        for bus in [BusTypeAta, BusTypeSata, BusTypeNvme] {
            assert_eq!(
                classify_descriptor(&descriptor(bus, b"Physical SSD model"), Some(&seek(0))),
                StorageKind::LocalSsd
            );
            assert_eq!(
                classify_descriptor(&descriptor(bus, b"Physical HDD model"), Some(&seek(1))),
                StorageKind::Rotational
            );
        }
        assert_eq!(
            classify_descriptor(&descriptor(BusTypeiScsi, b"Remote"), Some(&seek(0))),
            StorageKind::Network
        );
    }

    /// THUMB-UNKNOWN-STORAGE: a virtual or cloud machine's disk and a removable disk are
    /// attached to this machine, so they are `Local` of unknown media, never `Unknown`.
    #[test]
    fn virtual_machine_removable_and_controller_disks_are_local_of_unknown_media() {
        for (bus, model) in [
            (BusTypeUsb, b"SSD".as_slice()),
            (BusTypeSd, b"SD Card"),
            (BusTypeAta, b"QEMU HARDDISK"),
            (BusTypeSata, b"VBOX HARDDISK"),
            (BusTypeScsi, b"Virtual Disk"),
            (BusTypeSas, b"VMware Virtual disk"),
            (BusTypeNvme, b"Amazon Elastic Block Store"),
            (BusTypeNvme, b""),
        ] {
            assert_eq!(
                classify_descriptor(&descriptor(bus, model), Some(&seek(0))),
                StorageKind::Local,
                "{bus} {model:?}"
            );
        }
        let mut removable = descriptor(BusTypeSata, b"SSD");
        removable[offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia)] = 1;
        assert_eq!(
            classify_descriptor(&removable, Some(&seek(0))),
            StorageKind::Local
        );
        // A physical disk whose driver gives no seek-penalty answer.
        assert_eq!(
            classify_descriptor(&descriptor(BusTypeNvme, b"Physical SSD model"), None),
            StorageKind::Local
        );
    }

    #[test]
    fn usb_sd_and_removable_media_disks_are_removable_and_internal_disks_are_not() {
        for bus in [BusTypeUsb, BusTypeSd, BusTypeMmc] {
            assert_eq!(removable_descriptor(&descriptor(bus, b"Disk")), Some(true));
        }
        for bus in [
            BusTypeAta,
            BusTypeSata,
            BusTypeNvme,
            BusTypeVirtual,
            BusTypeiScsi,
        ] {
            assert_eq!(removable_descriptor(&descriptor(bus, b"Disk")), Some(false));
        }
        let mut media = descriptor(BusTypeSata, b"Disk");
        media[offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia)] = 1;
        assert_eq!(removable_descriptor(&media), Some(true));
        // A truncated descriptor tells nothing.
        assert_eq!(removable_descriptor(&media[..8]), None);
    }

    #[test]
    fn the_system_drive_is_neither_removable_nor_a_network_location() {
        let temp = std::env::temp_dir();
        assert!(!network(&temp));
        assert!(network(Path::new(r"\\server\share\folder")));
        // CI and development machines keep their temporary folder on a fixed disk.
        assert!(!removable(&temp), "{}", temp.display());
    }

    #[test]
    fn disks_that_may_be_elsewhere_prove_nothing_local() {
        for (bus, model) in [
            (BusTypeVirtual, b"Disk".as_slice()),
            (BusTypeFileBackedVirtual, b"Msft Virtual Disk"),
            (BusTypeSpaces, b"Storage Space"),
            (BusTypeUnknown, b"Disk"),
            (20, b"Disk"),
            (BusTypeScsi, b"iSCSI Disk"),
        ] {
            assert_eq!(
                classify_descriptor(&descriptor(bus, model), Some(&seek(0))),
                StorageKind::Unknown,
                "{bus} {model:?}"
            );
        }
    }

    #[test]
    fn truncated_or_invalid_driver_descriptors_are_unknown_without_bool_transmute() {
        let device = descriptor(BusTypeNvme, b"SSD");
        assert_eq!(
            classify_descriptor(&device[..8], Some(&seek(0))),
            StorageKind::Unknown
        );
        assert_eq!(
            classify_descriptor(&device, Some(&[])),
            StorageKind::Unknown
        );
        assert_eq!(
            classify_descriptor(&device, Some(&seek(2))),
            StorageKind::Unknown
        );
        let mut invalid = device.clone();
        invalid[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            classify_descriptor(&invalid, Some(&seek(0))),
            StorageKind::Unknown
        );
        invalid = device;
        invalid[offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia)] = 2;
        assert_eq!(
            classify_descriptor(&invalid, Some(&seek(0))),
            StorageKind::Unknown
        );
    }

    #[test]
    fn unc_guards_classify_network_without_accessing_nonexistent_server() {
        for path in [
            r"\\does-not-exist.invalid\share\文档",
            r"\\?\UNC\does-not-exist.invalid\share\👩🏽‍💻",
        ] {
            assert_eq!(detect(Path::new(path)), StorageKind::Network);
        }
    }

    #[test]
    fn native_temporary_volume_hint_does_not_write_directory_contents() {
        let directory = crate::test_fixtures::build_temp("fixture-");
        let _ = detect(directory.path());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
