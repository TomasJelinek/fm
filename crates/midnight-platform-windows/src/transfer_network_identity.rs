//! A Copy without a server file ID keeps the authority to read or publish in its
//! open handles. These process-local keys let separate preparations of a live
//! name correlate; they cannot prove that reopening a path found the same object.
use super::*;
use std::hash::{BuildHasher, RandomState};
use std::sync::OnceLock;

pub(super) fn is_network(file: &File, path: &Path) -> Result<bool, FsError> {
    #[cfg(feature = "test-fixtures")]
    if crate::test_network_ids::is_network(file) {
        return Ok(true);
    }
    // Ask about the handle, including a drive-letter path that traversed a junction.
    // The opened name suffices for finding the volume and avoids asking SMB to
    // normalize each parent just to find out whether this is a share.
    let actual = destination::final_path(file, VOLUME_NAME_DOS | FILE_NAME_OPENED)
        .map_err(|failure| error(path, failure))?;
    Ok(crate::storage_kind::network(&actual))
}

/// Whether Copy may read `file` without a file ID, through bindings that its open
/// handles keep: a network share, or a local volume whose file system has no 128-bit
/// file ID (FAT, FAT32 and exFAT memory cards and USB sticks, CD and DVD file systems).
/// Copy, checksums and drags from such a drive need it; Move still needs stable IDs.
pub(super) fn reads_without_ids(file: &File, path: &Path) -> Result<bool, FsError> {
    if is_network(file, path)? {
        return Ok(true);
    }
    let (_, _, filesystem) =
        super::link_destination::volume_facts(file).map_err(|failure| error(path, failure))?;
    Ok(id_less_filesystem(&filesystem))
}

/// The local file systems without a 128-bit file ID (MS-FSCC appendix, 2.1.10).
fn id_less_filesystem(name: &str) -> bool {
    matches!(name, "FAT" | "FAT32" | "exFAT" | "CDFS" | "UDF")
}

pub(super) fn binding(file: &File, path: &Path) -> Result<TransferIdentity, FsError> {
    let actual =
        destination::final_path(file, VOLUME_NAME_DOS).map_err(|failure| error(path, failure))?;
    static KEYS: OnceLock<[RandomState; 3]> = OnceLock::new();
    let keys = KEYS.get_or_init(|| std::array::from_fn(|_| RandomState::new()));
    let mut object = [0u8; 16];
    object[..8].copy_from_slice(&keys[0].hash_one(&actual).to_le_bytes());
    object[8..].copy_from_slice(&keys[1].hash_one(&actual).to_le_bytes());
    Ok(TransferIdentity {
        device: keys[2].hash_one(actual.components().next()),
        object,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_file_systems_without_file_ids_are_named_exactly() {
        for name in ["FAT", "FAT32", "exFAT", "CDFS", "UDF"] {
            assert!(super::id_less_filesystem(name), "{name}");
        }
        for name in ["NTFS", "ReFS", "fat32", "", "FAT32 "] {
            assert!(!super::id_less_filesystem(name), "{name:?}");
        }
    }
}
