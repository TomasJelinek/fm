//! Whether a typed Copy/Move destination is an existing folder, asked before the
//! dialog admits a job, and what its filesystem does with links, from which the dialog
//! chooses its default link handling. Advisory and read-only: the transfer checks
//! everything again.
use crate::{Cancellation, DesktopRequestId, FsError};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DestinationCheckRequest {
    pub request: DesktopRequestId,
    pub path: PathBuf,
    /// Also ask what the folder does with links (`DestinationCheckResult::links`).
    pub links: bool,
    /// The folder that holds the selection, for `TransferLinkFacts::same_volume`.
    pub source: Option<PathBuf>,
    pub cancel: Cancellation,
}

#[derive(Clone, Debug)]
pub struct DestinationCheckResult {
    pub request: DesktopRequestId,
    pub path: PathBuf,
    /// `Ok(true)`: an existing folder, links followed. `Ok(false)`: something else
    /// exists there. A missing path is an error of kind `NotFound`.
    pub result: Result<bool, FsError>,
    /// What an existing folder does with links, when the request asked and it could be
    /// read; None otherwise.
    pub links: Option<TransferLinkFacts>,
}

/// Whether a destination's filesystem stores links, as its volume reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferLinkStorage {
    /// The volume reports link support: reparse points on Windows, a filesystem
    /// known to store symbolic links on Linux.
    Supported,
    /// FAT32, exFAT and the like, and (on Windows) network locations, where the
    /// transfer creates no links.
    Unsupported,
    /// Nothing tells: the transfer tries and reports each link it cannot create.
    #[default]
    Unknown,
}

/// What a destination folder's volume reports about links (`TransferService::link_destination`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferLinkDestination {
    pub storage: TransferLinkStorage,
    /// The filesystem's name when it decides `storage` (for example "FAT32" or "exFAT").
    pub filesystem: Option<String>,
    /// A network location (a share or a mapped network drive).
    pub network: bool,
    /// Removable media, or a drive on a removable bus such as USB or an SD card reader.
    pub removable: bool,
    /// The volume's native identity, to tell whether a selection is on the same drive
    /// (`same_volume_as`): on Linux the mount (a bind mount is another one), on Windows
    /// the volume serial number, and for a network location its share's (the server
    /// volume's number with the share's name, so that two shares are two volumes).
    pub volume: Option<u64>,
    /// A Move from another drive cannot end here. The native adapters never set it: such
    /// a Move copies and verifies each item as Copy does and removes its source after that,
    /// so it goes wherever Copy goes. A provider whose destination cannot take one says so
    /// here, and the Copy/Move dialog, a drag and the Move job refuse it up front.
    pub refuses_cross_volume_move: bool,
}

impl TransferLinkDestination {
    /// Why a Move from another drive does not start here, in words for the user (what is
    /// refused, why, and that Copy is the way), or None where it can. The Copy/Move dialog
    /// shows it as a short note under its destination; a drag's refusal says the same.
    pub fn cross_volume_move_refusal(&self) -> Option<String> {
        if !self.refuses_cross_volume_move {
            return None;
        }
        let here = if self.network {
            "to a network location".to_owned()
        } else {
            match self.filesystem.as_deref() {
                Some(name) => format!("to this drive ({name})"),
                None => "to this drive".to_owned(),
            }
        };
        Some(format!(
            "Items from another drive cannot be moved {here}: Move cannot verify their copies \
             there before removing the originals. Copy them instead."
        ))
    }

    /// Whether a Move from the folder these facts describe to the folder `other`
    /// describes stays on one volume, where it renames its items, rather than copying
    /// them to another: the same volume, reached the same way, both locally or both
    /// through a network location. A share of a local drive (`\\localhost\D$`) can report
    /// the drive's own volume, yet a rename cannot pass between the drive and the share,
    /// so a Move there is a Move from another drive. An unknown volume is not the same.
    /// The one determination: the Copy/Move dialog's facts, a drop's admission and a Move
    /// job's admission all decide by it, so the dialog never offers a Move that an
    /// admission then refuses.
    pub fn same_volume_as(&self, other: &Self) -> bool {
        self.volume.is_some() && self.volume == other.volume && self.network == other.network
    }
}

/// Why the Copy/Move dialog copies the files links point to by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferLinkDefaultReason {
    /// The destination's filesystem cannot store links.
    CannotStoreLinks,
    /// The destination is a network location where links are not created.
    NetworkLocation,
    /// The destination is a removable drive that the selection is not already on.
    RemovableDrive,
}

/// What the dialog knows about its destination's links, and the selection's volume.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferLinkFacts {
    pub destination: TransferLinkDestination,
    /// The selection's folder is on the destination's volume, reached the same way
    /// (`TransferLinkDestination::same_volume_as`). A Move there renames and a link
    /// already lives on that drive, so a removable drive keeps links by default.
    pub same_volume: bool,
}

impl TransferLinkFacts {
    /// The facts of `destination` for a selection in the folder that `source` describes:
    /// None where no one folder holds the selection or its facts cannot be read, and the
    /// selection is then not known to share the destination's volume.
    pub fn new(
        destination: TransferLinkDestination,
        source: Option<&TransferLinkDestination>,
    ) -> Self {
        let same_volume = source.is_some_and(|source| source.same_volume_as(&destination));
        Self {
            destination,
            same_volume,
        }
    }

    /// Why the default is to copy the files links point to; None keeps links as links.
    /// A destination that cannot store links copies the files; so does a removable
    /// drive, where links would point back to the computer the files came from, unless
    /// the selection is already on that drive.
    pub fn copy_files_by_default(&self) -> Option<TransferLinkDefaultReason> {
        let destination = &self.destination;
        if destination.storage == TransferLinkStorage::Unsupported {
            return Some(if destination.network {
                TransferLinkDefaultReason::NetworkLocation
            } else {
                TransferLinkDefaultReason::CannotStoreLinks
            });
        }
        (destination.removable && !self.same_volume)
            .then_some(TransferLinkDefaultReason::RemovableDrive)
    }

    /// Keeping links would fail for every link: the destination cannot store them.
    pub fn cannot_keep_links(&self) -> bool {
        self.destination.storage == TransferLinkStorage::Unsupported
    }

    /// Why a Move of the selection does not start here: it would come from another drive
    /// (`same_volume` is false) to a destination that refuses such a Move.
    pub fn move_refusal(&self) -> Option<String> {
        if self.same_volume {
            return None;
        }
        self.destination.cross_volume_move_refusal()
    }
}

/// What the Copy/Move dialog asks when it opens or its destination changes, answered by
/// `Event::TransferLinksInspected` on a backend worker.
#[derive(Clone, Debug)]
pub struct TransferLinksRequest {
    pub request: DesktopRequestId,
    pub destination: PathBuf,
    /// The folder that holds the selection, when one folder holds it.
    pub source: Option<PathBuf>,
    pub cancel: Cancellation,
}

#[derive(Clone, Debug)]
pub struct TransferLinksResult {
    pub request: DesktopRequestId,
    pub destination: PathBuf,
    pub result: Result<TransferLinkFacts, FsError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(storage: TransferLinkStorage, network: bool, removable: bool) -> TransferLinkFacts {
        TransferLinkFacts {
            destination: TransferLinkDestination {
                storage,
                filesystem: None,
                network,
                removable,
                volume: None,
                refuses_cross_volume_move: false,
            },
            same_volume: false,
        }
    }

    #[test]
    fn links_stay_links_unless_the_destination_cannot_store_them_or_is_removable() {
        use TransferLinkDefaultReason::*;
        use TransferLinkStorage::*;
        assert_eq!(facts(Supported, false, false).copy_files_by_default(), None);
        assert_eq!(facts(Unknown, false, false).copy_files_by_default(), None);
        assert_eq!(
            facts(Unsupported, false, false).copy_files_by_default(),
            Some(CannotStoreLinks)
        );
        assert_eq!(
            facts(Unsupported, true, false).copy_files_by_default(),
            Some(NetworkLocation)
        );
        // A FAT32 stick: the filesystem decides before the bus.
        assert_eq!(
            facts(Unsupported, false, true).copy_files_by_default(),
            Some(CannotStoreLinks)
        );
        assert_eq!(
            facts(Supported, false, true).copy_files_by_default(),
            Some(RemovableDrive)
        );
        assert_eq!(
            facts(Unknown, false, true).copy_files_by_default(),
            Some(RemovableDrive)
        );
        assert!(facts(Unsupported, false, false).cannot_keep_links());
        assert!(!facts(Unknown, false, false).cannot_keep_links());
    }

    /// A Move from another drive is refused where the destination cannot take one, in
    /// words that say what, why and what to do instead; a Move on the destination's own
    /// drive is not, and where nothing tells, the job decides.
    #[test]
    fn a_refused_move_says_why_and_suggests_copy() {
        let mut network = facts(TransferLinkStorage::Unsupported, true, false);
        assert_eq!(
            network.move_refusal(),
            None,
            "nothing tells: the job decides"
        );
        network.destination.refuses_cross_volume_move = true;
        assert_eq!(
            network.move_refusal().as_deref(),
            Some(
                "Items from another drive cannot be moved to a network location: Move cannot \
                 verify their copies there before removing the originals. Copy them instead."
            )
        );
        let mut stick = facts(TransferLinkStorage::Unsupported, false, true);
        stick.destination.refuses_cross_volume_move = true;
        stick.destination.filesystem = Some("exFAT".into());
        assert_eq!(
            stick.move_refusal().as_deref(),
            Some(
                "Items from another drive cannot be moved to this drive (exFAT): Move cannot \
                 verify their copies there before removing the originals. Copy them instead."
            )
        );
        stick.same_volume = true;
        assert_eq!(
            stick.move_refusal(),
            None,
            "on its own drive a Move renames"
        );
    }

    /// A Move stays on one volume only where both folders name the same volume and reach
    /// it the same way: the loopback share of a drive reports the drive's own volume
    /// serial number, and a Move there from the drive is still a Move from another drive.
    #[test]
    fn a_share_of_the_selections_drive_is_another_volume() {
        let drive = TransferLinkDestination {
            storage: TransferLinkStorage::Supported,
            volume: Some(0xd0d0),
            ..Default::default()
        };
        let share = TransferLinkDestination {
            storage: TransferLinkStorage::Unsupported,
            network: true,
            volume: Some(0xd0d0),
            refuses_cross_volume_move: true,
            ..Default::default()
        };
        assert!(drive.same_volume_as(&drive));
        assert!(
            share.same_volume_as(&share),
            "within one share a Move renames"
        );
        assert!(!drive.same_volume_as(&share) && !share.same_volume_as(&drive));
        let other = TransferLinkDestination {
            volume: Some(0xe0e0),
            ..drive.clone()
        };
        assert!(!drive.same_volume_as(&other));
        let unknown = TransferLinkDestination {
            volume: None,
            ..drive.clone()
        };
        assert!(!unknown.same_volume_as(&unknown), "unknown is not the same");
        let to_share = TransferLinkFacts::new(share.clone(), Some(&drive));
        assert!(!to_share.same_volume);
        assert_eq!(
            to_share.move_refusal().as_deref(),
            Some(
                "Items from another drive cannot be moved to a network location: Move cannot \
                 verify their copies there before removing the originals. Copy them instead."
            )
        );
        assert!(TransferLinkFacts::new(share.clone(), Some(&share)).same_volume);
        assert!(!TransferLinkFacts::new(share, None).same_volume);
    }

    #[test]
    fn a_selection_already_on_the_removable_drive_keeps_its_links() {
        let mut facts = facts(TransferLinkStorage::Supported, false, true);
        facts.same_volume = true;
        assert_eq!(facts.copy_files_by_default(), None);
        // A drive that cannot store links copies the files wherever the selection is.
        facts.destination.storage = TransferLinkStorage::Unsupported;
        assert_eq!(
            facts.copy_files_by_default(),
            Some(TransferLinkDefaultReason::CannotStoreLinks)
        );
    }
}
