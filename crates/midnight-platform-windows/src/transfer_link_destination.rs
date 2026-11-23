//! What a Copy/Move destination folder's volume does with links, for the dialog's default
//! link handling: whether it supports reparse points (junctions and symbolic links), its
//! filesystem's name, and whether it is a network location or on removable media. Only
//! the volume's own reports are read; nothing is written to find out.
use super::*;
use midnight_core::{TransferLinkDestination, TransferLinkStorage};
use std::{
    os::windows::io::AsRawHandle,
    path::{Component, Prefix},
    ptr,
};
use windows::Win32::System::SystemServices::FILE_SUPPORTS_REPARSE_POINTS;
use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationByHandleW, VOLUME_NAME_DOS};

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
    // A Move from another drive goes wherever Copy goes: it copies and verifies each item
    // as Copy does and removes its source after that, so no destination refuses it.
    let refuses_cross_volume_move = false;
    let volume = if network {
        share_volume(&folder, u64::from(serial))
    } else {
        u64::from(serial)
    };
    checked(cancel, path)?;
    Ok(TransferLinkDestination {
        storage,
        // FAT32 and exFAT name themselves; a share's server filesystem is not shown.
        filesystem: (!network && !reparse_points && !name.is_empty()).then_some(name),
        network,
        removable,
        volume: Some(volume),
        refuses_cross_volume_move,
    })
}

/// The volume identity of `file`, reached through a network location and on a server
/// volume with the serial number `serial`. A share is a volume of its own: a rename
/// passes neither between two shares of one server volume nor between a local drive and
/// its share (`\\localhost\D$` reports D:'s serial number). So the share's name, as the
/// handle's final path gives it once a mapped drive is resolved, joins the serial number.
pub(super) fn share_volume(file: &File, serial: u64) -> u64 {
    let share = super::destination::final_path(file, VOLUME_NAME_DOS)
        .ok()
        .and_then(|path| share_root(&path));
    share_identity(serial, share.as_deref())
}

/// The share a final path reaches, `\\server\share` in lower case; None for a local
/// volume's path.
fn share_root(path: &Path) -> Option<String> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    match prefix.kind() {
        Prefix::VerbatimUNC(server, share) | Prefix::UNC(server, share) => Some(
            format!(
                r"\\{}\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            )
            .to_lowercase(),
        ),
        _ => None,
    }
}

/// A share's volume identity: its name (when known) and the server volume's serial
/// number, hashed (FNV-1a) into a value that is the same in every process.
fn share_identity(serial: u64, share: Option<&str>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in share
        .unwrap_or_default()
        .bytes()
        .chain(serial.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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

    /// A share is named by its server and share, whatever the case, from the final path
    /// of a handle (verbatim) or a plain UNC path; a local volume's path names none. Two
    /// shares of one server volume, and a share and the drive it shares, are distinct
    /// volumes; the same share is always the same one.
    #[test]
    fn a_share_is_a_volume_of_its_own() {
        for (path, share) in [
            (r"\\?\UNC\Server\D$\work\inbox", Some(r"\\server\d$")),
            (r"\\server\Share\inbox", Some(r"\\server\share")),
            (r"\\?\D:\work", None),
            (
                r"\\?\Volume{0b5e6a3d-0000-0000-0000-100000000000}\work",
                None,
            ),
            (r"D:\work", None),
        ] {
            assert_eq!(share_root(Path::new(path)).as_deref(), share, "{path}");
        }
        let serial = 0xd0d0_u64;
        let one = share_identity(serial, Some(r"\\server\a"));
        assert_eq!(one, share_identity(serial, Some(r"\\server\a")));
        assert_ne!(one, share_identity(serial, Some(r"\\server\b")));
        assert_ne!(one, serial, "not the shared drive's own number");
        assert_ne!(share_identity(serial, None), serial);
    }

    /// The drive's own administrative share (`\\localhost\X$`) reports the drive's volume
    /// serial number, yet a rename cannot pass between the drive and it: its facts name
    /// another volume, so a Move there from the drive is refused up front (O-1b), and a
    /// drag there proposes Copy. Needs the share, which a test machine may not grant.
    #[test]
    #[ignore = "needs the local drive's administrative share, \\\\localhost\\X$"]
    fn the_drives_own_loopback_share_is_another_volume_than_the_drive() {
        use midnight_core::{TransferLinkFacts, TransferService};
        let temp = std::env::temp_dir();
        let text = temp.to_string_lossy().into_owned();
        let (drive, rest) = text.split_at(2);
        assert!(drive.ends_with(':'), "a drive letter path: {text}");
        let share = std::path::PathBuf::from(format!(
            r"\\localhost\{}$\{}",
            &drive[..1],
            rest.trim_start_matches('\\')
        ));
        let cancel = Cancellation::default();
        let local = link_destination(&temp, &cancel).unwrap();
        let shared = link_destination(&share, &cancel)
            .unwrap_or_else(|error| panic!("{} is not reachable: {error:?}", share.display()));
        assert!(shared.network && !local.network, "{shared:?} {local:?}");
        assert_eq!(
            volume_facts(&native::open_parent(&share).unwrap())
                .unwrap()
                .0,
            volume_facts(&native::open_parent(&temp).unwrap())
                .unwrap()
                .0,
            "the share reports the drive's own serial number"
        );
        assert_ne!(shared.volume, local.volume);
        assert!(!local.same_volume_as(&shared));
        // A Move from the drive to its share copies, as Copy does; nothing refuses it.
        assert!(!shared.refuses_cross_volume_move, "{shared:?}");
        assert!(
            TransferLinkFacts::new(shared, Some(&local))
                .move_refusal()
                .is_none()
        );
        let filesystem = crate::WindowsFileSystem::new(None);
        assert_ne!(
            filesystem.native_drop_device(&share, &cancel).unwrap(),
            filesystem.native_drop_device(&temp, &cancel).unwrap(),
            "a drag from the drive to its share proposes Copy"
        );
    }
}
