//! Read-only details from one held native object and bounded kernel mount metadata.
use crate::{desktop::read_bounded, storage_kind};
use midnight_core::{
    Cancellation, FsError, StorageKind, VolumeCapacity, VolumeInformation, VolumeProperty,
};
use rustix::fs::{
    AtFlags, CWD, Mode, OFlags, StatVfsMountFlags, StatxFlags, fstatvfs, open, statx,
};
use std::{
    ffi::OsString,
    io,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

pub(crate) fn query(path: &Path, cancel: &Cancellation) -> Result<VolumeInformation, FsError> {
    let fail = |error: io::Error| FsError::from_io("query volume information", path, error);
    cancelled(cancel).map_err(fail)?;
    if !path.is_absolute() {
        return Err(fail(io::ErrorKind::InvalidInput.into()));
    }
    // O_PATH pins identity without reading file/device/FIFO content or requiring a reader.
    let file = open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty())
        .map_err(|error| fail(error.into()))?;
    let identity = statx(
        &file,
        "",
        AtFlags::EMPTY_PATH,
        StatxFlags::BASIC_STATS | StatxFlags::MNT_ID,
    )
    .map_err(|error| fail(error.into()))?;
    let mut information = VolumeInformation::default();
    cancelled(cancel).map_err(fail)?;
    match fstatvfs(&file) {
        Ok(stats) => {
            let unit = if stats.f_frsize == 0 {
                stats.f_bsize
            } else {
                stats.f_frsize
            };
            information.capacity = capacity(stats.f_blocks, stats.f_bfree, stats.f_bavail, unit);
            if unit == 0
                || information.capacity.total.is_none()
                || information.capacity.free.is_none()
                || information.capacity.available.is_none()
            {
                information
                    .unavailable
                    .push("Capacity: invalid allocation unit or native counter overflow".into());
            }
            information.read_only = Some(stats.f_flag.contains(StatVfsMountFlags::RDONLY));
            for (flag, property) in [
                (StatVfsMountFlags::NOEXEC, VolumeProperty::NoExecute),
                (StatVfsMountFlags::NOSUID, VolumeProperty::NoSetId),
                (StatVfsMountFlags::NODEV, VolumeProperty::NoDeviceFiles),
            ] {
                if stats.f_flag.contains(flag) {
                    information.properties.push(property);
                }
            }
        }
        Err(error) => information
            .unavailable
            .push(format!("Capacity and mount flags: {error}")),
    }
    cancelled(cancel).map_err(fail)?;
    let mount_id =
        (identity.stx_mask & StatxFlags::MNT_ID.bits() != 0).then_some(identity.stx_mnt_id);
    if let Some(mount) = mount_id.and_then(|id| {
        let bytes = read_bounded(Path::new("/proc/self/mountinfo"), 1024 * 1024)?;
        parse_mount(&bytes, id, identity.stx_dev_major, identity.stx_dev_minor)
    }) {
        information.root = Some(mount.root);
        information.filesystem = Some(mount.filesystem);
        information.source = Some(mount.source);
    } else {
        information
            .unavailable
            .push("Mount and filesystem identity unavailable from kernel metadata".into());
    }
    if identity.stx_dev_major != 0 {
        let database = PathBuf::from(format!(
            "/run/udev/data/b{}:{}",
            identity.stx_dev_major, identity.stx_dev_minor
        ));
        information.label = read_bounded(&database, 64 * 1024).and_then(|bytes| label(&bytes));
    }
    cancelled(cancel).map_err(fail)?;
    let storage = storage_kind::detect(path);
    // Classification uses native path queries. Reject it if that path no longer
    // refers to the held mount; capacities and mount metadata still use its identity.
    if let Ok(current) = statx(CWD, path, AtFlags::empty(), StatxFlags::MNT_ID)
        && mount_id == Some(current.stx_mnt_id)
        && current.stx_mask & StatxFlags::MNT_ID.bits() != 0
    {
        information.storage = storage;
    } else {
        information.storage = StorageKind::Unknown;
    }
    cancelled(cancel).map_err(fail)?;
    Ok(information)
}

fn capacity(total: u64, free: u64, available: u64, unit: u64) -> VolumeCapacity {
    if unit == 0 {
        return VolumeCapacity::default();
    }
    VolumeCapacity {
        total: total.checked_mul(unit),
        free: free.checked_mul(unit),
        available: available.checked_mul(unit),
    }
}

struct Mount {
    root: PathBuf,
    filesystem: OsString,
    source: OsString,
}

fn parse_mount(input: &[u8], id: u64, major: u32, minor: u32) -> Option<Mount> {
    if input.len() > 1024 * 1024 {
        return None;
    }
    let device = format!("{major}:{minor}");
    let mut found = None;
    for (index, line) in input.split(|byte| *byte == b'\n').enumerate() {
        if index >= 8192 {
            return None;
        }
        let Some(separator) = line.windows(3).position(|part| part == b" - ") else {
            continue;
        };
        let fields: Vec<_> = line[..separator]
            .split(|byte| *byte == b' ')
            .take(7)
            .collect();
        if fields.len() < 6 || std::str::from_utf8(fields[0]).ok()?.parse::<u64>().ok()? != id {
            continue;
        }
        if fields[2] != device.as_bytes() || found.is_some() {
            return None;
        }
        let root = PathBuf::from(OsString::from_vec(unescape(fields[4])?));
        if !root.is_absolute() {
            return None;
        }
        let mut extra = line[separator + 3..].split(|byte| *byte == b' ');
        let filesystem = OsString::from_vec(unescape(extra.next()?)?);
        let source = OsString::from_vec(unescape(extra.next()?)?);
        if filesystem.is_empty() || source.is_empty() {
            return None;
        }
        found = Some(Mount {
            root,
            filesystem,
            source,
        });
    }
    found
}

fn unescape(input: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'\\' {
            let octal = input.get(index + 1..index + 4)?;
            if !(b'0'..=b'3').contains(&octal[0])
                || !octal[1..].iter().all(|b| (b'0'..=b'7').contains(b))
            {
                return None;
            }
            decoded.push((octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + (octal[2] - b'0'));
            index += 4;
        } else {
            decoded.push(input[index]);
            index += 1;
        }
    }
    (!decoded.contains(&0)).then_some(decoded)
}

fn label(input: &[u8]) -> Option<OsString> {
    let mut labels = input
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"E:ID_FS_LABEL_ENC="));
    let encoded = labels.next()?;
    if labels.next().is_some() || encoded.is_empty() {
        return None;
    }
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'\\' {
            if encoded.get(index + 1) != Some(&b'x') {
                return None;
            }
            let hex = std::str::from_utf8(encoded.get(index + 2..index + 4)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 4;
        } else {
            decoded.push(encoded[index]);
            index += 1;
        }
    }
    (!decoded.contains(&0)).then(|| OsString::from_vec(decoded))
}

fn cancelled(cancel: &Cancellation) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn mount_selection_uses_exact_held_id_and_device_not_a_guessed_path_prefix() {
        let input = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /mnt/space\\040raw\\377 rw - xfs /dev/sdb1 rw\n3 1 8:3 / /mnt/space\\040raw\\377 rw - btrfs /dev/sdc1 rw\n";
        let found = parse_mount(input, 2, 8, 2).unwrap();
        assert_eq!(found.root.as_os_str().as_bytes(), b"/mnt/space raw\xff");
        assert_eq!(found.filesystem, "xfs");
        assert_eq!(found.source, "/dev/sdb1");
        assert!(parse_mount(input, 2, 8, 99).is_none());
        assert!(parse_mount(input, 99, 8, 2).is_none());
    }

    #[test]
    fn udev_label_decoding_preserves_unicode_raw_bytes_and_rejects_bad_escapes() {
        assert_eq!(
            label(b"E:OTHER=ignored\nE:ID_FS_LABEL_ENC=Disk\\x20\\xe6\\x96\\x87\\xff\n")
                .unwrap()
                .as_bytes(),
            b"Disk \xe6\x96\x87\xff"
        );
        for input in [
            b"E:ID_FS_LABEL_ENC=bad\\xzz".as_slice(),
            b"E:ID_FS_LABEL_ENC=bad\\x00",
            b"E:ID_FS_LABEL_ENC=x\nE:ID_FS_LABEL_ENC=y",
        ] {
            assert!(label(input).is_none());
        }
    }

    #[test]
    fn an_invalid_native_counter_does_not_discard_other_measured_capacity() {
        let measured = capacity(10, 4, u64::MAX, 4096);
        assert_eq!(measured.total, Some(40_960));
        assert_eq!(measured.free, Some(16_384));
        assert_eq!(measured.used(), Some(24_576));
        assert_eq!(measured.available, None);
        assert_eq!(capacity(10, 4, 2, 0), VolumeCapacity::default());
        assert_eq!(capacity(u64::MAX, 4, 2, 4096).free, Some(16_384));
    }

    #[test]
    fn native_volume_query_reports_real_mount_and_capacity_without_reading_contents() {
        let fixture = tempfile::tempdir().unwrap();
        let directory = fixture.path().join("Žluťoučký_文档_🦀");
        std::fs::create_dir(&directory).unwrap();
        let details = query(&directory, &Cancellation::default()).unwrap();
        assert!(details.root.as_ref().unwrap().is_absolute());
        assert!(!details.filesystem.as_ref().unwrap().is_empty());
        assert!(details.capacity.total.unwrap() > 0);
        assert!(details.capacity.free.unwrap() <= details.capacity.total.unwrap());
        assert!(details.capacity.available.is_some());
        let raw = directory.join(OsString::from_vec(b"raw_\xff".to_vec()));
        std::fs::create_dir(&raw).unwrap();
        assert!(query(&raw, &Cancellation::default()).is_ok());
        let mut long = directory.clone();
        for _ in 0..4 {
            long = long.join("long_資料_".repeat(12));
        }
        std::fs::create_dir_all(&long).unwrap();
        assert!(long.as_os_str().as_bytes().len() > 260);
        assert!(query(&long, &Cancellation::default()).is_ok());
        assert!(query(&directory.join("missing"), &Cancellation::default()).is_err());
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert_eq!(
            query(&directory, &cancelled).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
    }
}
