//! Typed metadata snapshots; capturing these values performs no provider work.
use midnight_core::{ArchiveAddress, ArchiveMount, ArchiveNode};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub enum ThumbnailContent {
    Native(PathBuf),
    /// System artwork for an executable, handled by the desktop actor, never the image decoder.
    ExecutableIcon(PathBuf),
    Archive {
        mount: Arc<dyn ArchiveMount>,
        node: ArchiveNode,
        address: Arc<ArchiveAddress>,
        /// Backend admission checks the actual container, not a previous panel's storage hint.
        network_allowed: bool,
    },
}

impl PartialEq for ThumbnailContent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(left), Self::Native(right))
            | (Self::ExecutableIcon(left), Self::ExecutableIcon(right)) => left == right,
            (
                Self::Archive {
                    node: left,
                    address: left_address,
                    network_allowed: left_network,
                    ..
                },
                Self::Archive {
                    node: right,
                    address: right_address,
                    network_allowed: right_network,
                    ..
                },
            ) => {
                // Mount IDs scope live authority; persistent cache identity is computed and
                // validated separately by the backend. Captions are never identity keys.
                left.id == right.id
                    && left.parent == right.parent
                    && left.raw_name == right.raw_name
                    && left.kind == right.kind
                    && left.readable == right.readable
                    && left.size == right.size
                    && left_address == right_address
                    && left_network == right_network
            }
            _ => false,
        }
    }
}
impl Eq for ThumbnailContent {}
