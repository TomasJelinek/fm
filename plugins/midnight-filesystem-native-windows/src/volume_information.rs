//! Explicit volume metadata, with dynamic UTF-16 buffers and no media-error dialogs.
use crate::{storage_kind, volume_details};
use midnight_filesystem_plugin_api::{
    Address, AddressEncoding, Failure, VolumeCapacity, VolumeInformation, VolumeProperty,
};
use midnight_filesystem_plugin_sdk::CallContext;
use std::{io, path::Path, ptr};
use windows_sys::Win32::Storage::FileSystem::{
    DISK_SPACE_INFORMATION, GetDiskFreeSpaceExW, GetDiskSpaceInformationW, GetVolumeInformationW,
};
use windows_sys::Win32::System::SystemServices::*;

pub(crate) fn query(path: &Path, cancel: &CallContext<'_>) -> Result<VolumeInformation, Failure> {
    let fail = Failure::from_io;
    let _mode = volume_details::ErrorMode::new().map_err(fail)?;
    cancelled(cancel).map_err(fail)?;
    let root = volume_details::root(path, cancel)?;
    let mut information = VolumeInformation {
        root: Some(Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes: volume_details::text(&root[..root.len() - 1]).bytes,
        }),
        ..Default::default()
    };
    let (mut serial, mut flags) = (0, 0);
    match volume_details::wide_buffer(cancel, |output| {
        let length = u32::try_from(output.len())
            .map_err(|_| io::Error::other("Volume label buffer overflow"))?;
        // SAFETY: root is terminated; output and scalar pointers are valid for this call.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                output.as_mut_ptr(),
                length,
                &mut serial,
                ptr::null_mut(),
                &mut flags,
                ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }) {
        Ok(label) => {
            information.label = (!label.is_empty()).then(|| volume_details::text(&label));
            information.source = Some(volume_details::text(
                &format!("{:04X}-{:04X}", serial >> 16, serial & 0xffff)
                    .encode_utf16()
                    .collect::<Vec<_>>(),
            ));
            information.read_only = Some(flags & FILE_READ_ONLY_VOLUME != 0);
            information.properties = properties(flags);
        }
        Err(error) => information
            .unavailable
            .push(format!("Volume label and properties: {error}")),
    }
    cancelled(cancel).map_err(fail)?;
    match volume_details::wide_buffer(cancel, |output| {
        let length = u32::try_from(output.len())
            .map_err(|_| io::Error::other("Filesystem name buffer overflow"))?;
        // SAFETY: root and output have valid lengths; all unused pointers are null.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                output.as_mut_ptr(),
                length,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }) {
        Ok(name) => {
            information.filesystem = (!name.is_empty()).then(|| volume_details::text(&name))
        }
        Err(error) => information
            .unavailable
            .push(format!("Filesystem name: {error}")),
    }
    cancelled(cancel).map_err(fail)?;
    information.capacity = capacity(&root, &mut information.unavailable, cancel);
    cancelled(cancel).map_err(fail)?;
    information.storage = storage_kind::detect(path, &|| cancel.is_cancelled());
    cancelled(cancel).map_err(fail)?;
    Ok(information)
}

fn capacity(
    root: &[u16],
    unavailable: &mut Vec<String>,
    cancel: &CallContext<'_>,
) -> VolumeCapacity {
    let mut capacity = VolumeCapacity::default();
    let (mut available, mut free) = (0, 0);
    // SAFETY: terminated root and writable u64 outputs. Deliberately omit the
    // quota-limited total output; it is not the actual filesystem capacity.
    if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, ptr::null_mut(), &mut free) }
        != 0
    {
        capacity.available = Some(available);
        capacity.free = Some(free);
    } else {
        unavailable.push(format!("Free space: {}", io::Error::last_os_error()));
    }
    if cancel.is_cancelled() {
        return capacity;
    }
    let mut measured = DISK_SPACE_INFORMATION::default();
    // SAFETY: writable SDK structure and terminated root. This API is available
    // before the project's Windows 10 22H2 minimum and reports quota-independent total.
    let status = unsafe { GetDiskSpaceInformationW(root.as_ptr(), &mut measured) };
    if status >= 0 {
        let unit = u64::from(measured.SectorsPerAllocationUnit)
            .checked_mul(u64::from(measured.BytesPerSector));
        capacity.total = unit
            .filter(|unit| *unit != 0)
            .and_then(|unit| measured.ActualTotalAllocationUnits.checked_mul(unit));
        if capacity.total.is_none() {
            unavailable
                .push("Total space: native capacity overflow or invalid allocation unit".into());
        }
    } else {
        unavailable.push(format!(
            "Actual total space: Windows error 0x{:08X}",
            status as u32
        ));
    }
    capacity
}

fn properties(flags: u32) -> Vec<VolumeProperty> {
    [
        (
            FILE_CASE_PRESERVED_NAMES,
            VolumeProperty::CasePreservedNames,
        ),
        (
            FILE_CASE_SENSITIVE_SEARCH,
            VolumeProperty::CaseSensitiveLookup,
        ),
        (FILE_UNICODE_ON_DISK, VolumeProperty::UnicodeNames),
        (FILE_PERSISTENT_ACLS, VolumeProperty::PersistentAcls),
        (FILE_FILE_COMPRESSION, VolumeProperty::FileCompression),
        (FILE_SUPPORTS_SPARSE_FILES, VolumeProperty::SparseFiles),
        (FILE_NAMED_STREAMS, VolumeProperty::NamedStreams),
        (FILE_SUPPORTS_HARD_LINKS, VolumeProperty::HardLinks),
        (FILE_SUPPORTS_REPARSE_POINTS, VolumeProperty::ReparsePoints),
        (FILE_SUPPORTS_ENCRYPTION, VolumeProperty::Encryption),
        (FILE_VOLUME_QUOTAS, VolumeProperty::Quotas),
    ]
    .into_iter()
    .filter_map(|(flag, property)| (flags & flag != 0).then_some(property))
    .collect()
}

fn cancelled(cancel: &CallContext<'_>) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_properties_are_only_the_reported_native_flags() {
        assert!(properties(0).is_empty());
        assert_eq!(
            properties(FILE_NAMED_STREAMS | FILE_SUPPORTS_SPARSE_FILES),
            [VolumeProperty::SparseFiles, VolumeProperty::NamedStreams]
        );
    }
}
