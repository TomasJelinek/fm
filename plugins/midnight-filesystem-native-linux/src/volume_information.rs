//! Read-only volume details from a held native object and bounded kernel metadata.
use crate::{
    storage_kind, system,
    volume_details::{read_bounded, text},
};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeAdapter};
use std::{
    ffi::OsString,
    fs::File,
    os::{fd::AsRawFd, unix::ffi::OsStringExt},
    path::{Path, PathBuf},
};

pub fn identity(file: &File) -> Result<libc::statx, Failure> {
    // SAFETY: initialized plain native output, retained fd and static empty name.
    let mut value: libc::statx = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH,
            libc::STATX_BASIC_STATS | libc::STATX_MNT_ID,
            &mut value,
        )
    } != 0
    {
        return Err(Failure::from_io(std::io::Error::last_os_error()));
    }
    Ok(value)
}
pub fn query(path: &Path, context: &CallContext<'_>) -> Result<VolumeInformation, Failure> {
    let file = system::open_path(path, libc::O_PATH, context)?;
    let held = identity(&file)?;
    let mut info = VolumeInformation::default();
    system::cancel(context)?;
    match system::statvfs(&file) {
        Ok(stats) => {
            let unit = if stats.f_frsize == 0 {
                stats.f_bsize
            } else {
                stats.f_frsize
            };
            info.capacity = capacity(stats.f_blocks, stats.f_bfree, stats.f_bavail, unit);
            if info.capacity.total.is_none()
                || info.capacity.free.is_none()
                || info.capacity.available.is_none()
            {
                info.unavailable
                    .push("Capacity: invalid native counters or allocation unit".into());
            }
            info.read_only = Some(stats.f_flag & libc::ST_RDONLY != 0);
            for (flag, property) in [
                (libc::ST_NOEXEC, VolumeProperty::NoExecute),
                (libc::ST_NOSUID, VolumeProperty::NoSetId),
                (libc::ST_NODEV, VolumeProperty::NoDeviceFiles),
            ] {
                if stats.f_flag & flag != 0 {
                    info.properties.push(property);
                }
            }
        }
        Err(error) => info
            .unavailable
            .push(format!("Capacity and mount flags: {error}")),
    }
    system::cancel(context)?;
    let mount_id = (held.stx_mask & libc::STATX_MNT_ID != 0).then_some(held.stx_mnt_id);
    if let Some(mount) = mount_id.and_then(|id| {
        let input = read_bounded(Path::new("/proc/self/mountinfo"), 1024 * 1024, &|| {
            context.is_cancelled()
        })?;
        parse_mount(&input, id, held.stx_dev_major, held.stx_dev_minor)
    }) {
        info.root = Some(crate::adapter::Linux::encode(&mount.root));
        info.filesystem = Some(text(&mount.filesystem));
        info.source = Some(text(&mount.source));
    } else {
        info.unavailable
            .push("Mount and filesystem identity unavailable from kernel metadata".into());
    }
    system::cancel(context)?;
    info.label = device_label(held.stx_dev_major, held.stx_dev_minor, context);
    system::cancel(context)?;
    let storage = storage_kind::detect(path, &|| context.is_cancelled());
    // Path-based classification is valid only while it still names the held mount.
    if let Ok(current) =
        system::open_path(path, libc::O_PATH, context).and_then(|file| identity(&file))
        && current.stx_mask & libc::STATX_MNT_ID != 0
        && mount_id == Some(current.stx_mnt_id)
        && (held.stx_dev_major, held.stx_dev_minor)
            == (current.stx_dev_major, current.stx_dev_minor)
    {
        info.storage = storage;
    }
    system::cancel(context)?;
    Ok(info)
}
pub fn device_label(major: u32, minor: u32, context: &CallContext<'_>) -> Option<NativeText> {
    if major == 0 {
        return None;
    }
    let database = PathBuf::from(format!("/run/udev/data/b{major}:{minor}"));
    let input = read_bounded(&database, 64 * 1024, &|| context.is_cancelled())?;
    label(&input).map(|label| text(&label))
}
fn capacity(total: u64, free: u64, available: u64, unit: u64) -> VolumeCapacity {
    if unit == 0 {
        return VolumeCapacity::default();
    }
    VolumeCapacity {
        total: total.checked_mul(unit),
        free: free.checked_mul(unit).filter(|_| free <= total),
        available: available.checked_mul(unit).filter(|_| available <= total),
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
        assert_eq!(
            measured
                .total
                .zip(measured.free)
                .map(|(total, free)| total - free),
            Some(24_576)
        );
        assert_eq!(measured.available, None);
        assert_eq!(capacity(10, 4, 2, 0), VolumeCapacity::default());
        assert_eq!(capacity(u64::MAX, 4, 2, 4096).free, Some(16_384));
    }
}
