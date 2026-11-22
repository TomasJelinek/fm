//! Destinations of a Move from a folder on the local drive D:, as a Windows provider
//! reports them (test-only): a folder on the same drive, another local NTFS volume, a
//! FAT32 stick, the drive's own loopback share (`\\localhost\D$`, which reports D:'s
//! volume serial number although a rename from D: cannot reach it) and a share on
//! another computer. The paths are made under the temporary folder, so they are absolute
//! on every platform; the first component below it names the place. Reading "cancelled"
//! cancels the check, and any other place cannot be read.
use midnight_core::{
    Cancellation, CopyLinkPolicy, FsError, PreparedTransferMoveEntry, TransferDirectory,
    TransferLinkDestination, TransferLinkStorage, TransferRevisionPolicy, TransferService,
    TransferSource,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The volume serial numbers the places report.
const DRIVE_D: u64 = 0xd0d0;
const DRIVE_E: u64 = 0xe0e0;
const STICK_F: u64 = 0xf0f0;
const SERVER: u64 = 0x5e5e;

/// What a Move job's preparation answers once its admission let the Move through: it
/// would now capture the sources, which these tests never do.
pub(crate) const ADMITTED: &str = "admitted: the Move would now capture its sources";

const NETWORK_REFUSAL: &str = "Items from another drive cannot be moved to a network location: \
     Move cannot verify their copies there before removing the originals. Copy them instead.";
const STICK_REFUSAL: &str = "Items from another drive cannot be moved to this drive (FAT32): \
     Move cannot verify their copies there before removing the originals. Copy them instead.";

pub(crate) fn place(name: &str) -> PathBuf {
    std::env::temp_dir().join("midnight-move-places").join(name)
}

/// The folder the selection is in, on D:.
pub(crate) fn source_folder() -> PathBuf {
    place("drive-d").join("work")
}

/// A Move destination and what a Move from `source_folder` there must do: start (None)
/// or be refused before any job, in these words.
pub(crate) struct Case {
    pub(crate) name: &'static str,
    pub(crate) destination: PathBuf,
    pub(crate) refusal: Option<&'static str>,
}

pub(crate) fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "a folder on the same drive",
            destination: place("drive-d").join("inbox"),
            refusal: None,
        },
        Case {
            name: "another local NTFS volume",
            destination: place("drive-e").join("inbox"),
            refusal: None,
        },
        Case {
            name: "a FAT32 stick",
            destination: place("stick-f").join("inbox"),
            refusal: Some(STICK_REFUSAL),
        },
        Case {
            name: "the drive's own loopback share",
            destination: place("loopback-d").join("inbox"),
            refusal: Some(NETWORK_REFUSAL),
        },
        Case {
            name: "a share on another computer",
            destination: place("server").join("inbox"),
            refusal: Some(NETWORK_REFUSAL),
        },
    ]
}

fn place_name(path: &Path) -> Option<String> {
    let rest = path
        .strip_prefix(std::env::temp_dir().join("midnight-move-places"))
        .ok()?;
    Some(rest.components().next()?.as_os_str().to_str()?.to_owned())
}

pub(crate) struct Places;

impl TransferService for Places {
    fn link_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<TransferLinkDestination, FsError> {
        let local = |volume| TransferLinkDestination {
            storage: TransferLinkStorage::Supported,
            volume: Some(volume),
            ..Default::default()
        };
        let share = |volume| TransferLinkDestination {
            storage: TransferLinkStorage::Unsupported,
            network: true,
            volume: Some(volume),
            refuses_cross_volume_move: true,
            ..Default::default()
        };
        match place_name(path).as_deref() {
            Some("drive-d") => Ok(local(DRIVE_D)),
            Some("drive-e") => Ok(local(DRIVE_E)),
            Some("stick-f") => Ok(TransferLinkDestination {
                storage: TransferLinkStorage::Unsupported,
                filesystem: Some("FAT32".into()),
                removable: true,
                volume: Some(STICK_F),
                refuses_cross_volume_move: true,
                ..Default::default()
            }),
            // The loopback share reports the serial number of the drive it shares.
            Some("loopback-d") => Ok(share(DRIVE_D)),
            Some("server") => Ok(share(SERVER)),
            // The check is cancelled while this place is read: the read fails as reads do.
            Some("cancelled") => {
                cancel.cancel();
                Err(FsError::from_io(
                    "inspect links",
                    path,
                    io::ErrorKind::PermissionDenied.into(),
                ))
            }
            _ => Err(FsError::from_io(
                "inspect links",
                path,
                io::ErrorKind::NotFound.into(),
            )),
        }
    }

    /// The volume serial number, as the Windows provider reports it for a drop: the
    /// loopback share's is D:'s own.
    fn native_drop_device(&self, path: &Path, _: &Cancellation) -> Result<Option<u64>, FsError> {
        Ok(match place_name(path).as_deref() {
            Some("drive-d" | "loopback-d") => Some(DRIVE_D),
            Some("drive-e") => Some(DRIVE_E),
            Some("stick-f") => Some(STICK_F),
            Some("server") => Some(SERVER),
            _ => None,
        })
    }

    fn prepare_move_source_with_policy(
        &self,
        path: &Path,
        _: CopyLinkPolicy,
        _: TransferRevisionPolicy,
        _: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        Err(FsError::from_io(
            "prepare Move source",
            path,
            io::Error::other(ADMITTED),
        ))
    }

    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("deciding whether a Move may start reads no source")
    }

    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("deciding whether a Move may start prepares no Copy destination")
    }
}
