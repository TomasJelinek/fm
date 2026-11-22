//! What a Copy/Move destination folder's volume does with links, for the dialog's default
//! link handling: whether it supports reparse points (junctions and symbolic links), its
//! filesystem's name, and whether it is a network location or on removable media. Only
//! the volume's own reports are read; nothing is written to find out.
use super::*;
use midnight_core::{TransferLinkDestination, TransferLinkStorage};
use std::{os::windows::io::AsRawHandle, ptr};
use windows::Win32::System::SystemServices::FILE_SUPPORTS_REPARSE_POINTS;
use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

pub(super) fn link_destination(
    path: &Path,
    cancel: &Cancellation,
) -> Result<TransferLinkDestination, FsError> {
    checked(cancel, path)?;
    validate_path(path, true)?;
    // No link is created at a network location: a junction's target would be resolved on
    // the server, and a symbolic link's by each client's evaluation policy.
    let network = crate::storage_kind::network(path);
    let folder = native::open_parent(path).map_err(|e| error(path, e))?;
    let (serial, flags, name) = volume_facts(&folder).map_err(|e| error(path, e))?;
    let reparse_points = flags & FILE_SUPPORTS_REPARSE_POINTS != 0;
    let storage = if network || !reparse_points {
        TransferLinkStorage::Unsupported
    } else {
        TransferLinkStorage::Supported
    };
    let removable = !network && crate::storage_kind::removable(path);
    // The same volume check that a Move from another drive makes before it removes
    // anything (`TransferCapabilities::recoverable_replacement`).
    let refuses_cross_volume_move = !super::destination::recovery::supported_volume(&folder);
    checked(cancel, path)?;
    Ok(TransferLinkDestination {
        storage,
        // FAT32 and exFAT name themselves; a share's server filesystem is not shown.
        filesystem: (!network && !reparse_points && !name.is_empty()).then_some(name),
        network,
        removable,
        volume: Some(u64::from(serial)),
        refuses_cross_volume_move,
    })
}

/// The volume serial number, filesystem flags and filesystem name of `folder`'s volume.
pub(super) fn volume_facts(folder: &File) -> io::Result<(u32, u32, String)> {
    let (mut serial, mut flags) = (0u32, 0u32);
    let mut name = [0u16; 64];
    // SAFETY: a live directory handle; the scalar outputs and the name buffer are valid
    // for their stated sizes; the volume-label output is not requested.
    if unsafe {
        GetVolumeInformationByHandleW(
            folder.as_raw_handle(),
            ptr::null_mut(),
            0,
            &mut serial,
            ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    Ok((serial, flags, String::from_utf16_lossy(&name[..length])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_ntfs_folder_stores_links_and_names_its_volume() {
        let temp = std::env::temp_dir();
        let facts = link_destination(&temp, &Cancellation::default()).unwrap();
        // Development and CI machines keep the temporary folder on a fixed NTFS volume.
        assert_eq!(facts.storage, TransferLinkStorage::Supported, "{facts:?}");
        assert!(!facts.network && !facts.removable, "{facts:?}");
        // A fixed NTFS volume can receive a verified Move from another drive.
        assert!(!facts.refuses_cross_volume_move, "{facts:?}");
        assert_eq!(facts.filesystem, None);
        let again = link_destination(&temp.join(".."), &Cancellation::default());
        assert!(again.is_err(), "a dot component is refused, not resolved");
        let folder = native::open_parent(&temp).unwrap();
        assert_eq!(
            facts.volume,
            Some(u64::from(volume_facts(&folder).unwrap().0))
        );
    }

    #[test]
    fn cancellation_and_a_missing_folder_stop_the_query() {
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            link_destination(&std::env::temp_dir(), &cancel)
                .unwrap_err()
                .kind,
            io::ErrorKind::Interrupted
        );
        let missing = std::env::temp_dir().join("midnight-no-such-destination-folder");
        assert_eq!(
            link_destination(&missing, &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::NotFound
        );
    }
}
