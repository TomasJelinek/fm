//! Lossless native, archive and Network Hot Paths; display text is never a locator.
use crate::{ArchiveAddress, NetworkAddress};
use std::path::{Path, PathBuf};

/// Stable callback identity, independent of names and displayed paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BookmarkId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub id: BookmarkId,
    /// Retained for compatibility; the manager presents the location instead.
    pub name: String,
    /// Native folder, or the native return location for an archive/Network address.
    pub path: PathBuf,
    pub archive: Option<ArchiveAddress>,
    /// Provider-owned discovery address; never a native path or authentication state.
    pub network: Option<NetworkAddress>,
    /// 1 through 9, and 10 for the zero key.
    pub slot: Option<u8>,
}

pub const MAX_BOOKMARKS: usize = 64;
pub const MAX_BOOKMARK_NAME_BYTES: usize = 512;

impl Bookmark {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.0 == 0 {
            return Err("Bookmark identity must be nonzero.");
        }
        if self.name.trim().is_empty()
            || self.name.len() > MAX_BOOKMARK_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("Bookmark name must contain 1 to 512 UTF-8 bytes without controls.");
        }
        if !self.path.is_absolute() || self.path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err("Bookmark path must be an absolute native path without NUL.");
        }
        if self.archive.is_some() && self.network.is_some() {
            return Err("A bookmark cannot contain both archive and Network addresses.");
        }
        if let Some(network) = &self.network {
            network.validate()?;
        }
        if let Some(archive) = &self.archive {
            archive.validate()?;
            if archive.container.parent() != Some(self.path.as_path()) {
                return Err("Archive bookmark return path must be its container's parent.");
            }
        }
        if self.slot.is_some_and(|slot| !(1..=10).contains(&slot)) {
            return Err("Bookmark hot path slot must be between 1 and 10.");
        }
        Ok(())
    }

    /// Lexical location equality, without filesystem access or lossy conversion.
    /// A newer revision of the same archive does not create a second Hot Path.
    pub fn matches_location(
        &self,
        path: &Path,
        archive: Option<&ArchiveAddress>,
        network: Option<&NetworkAddress>,
    ) -> bool {
        if self.network.is_some() || network.is_some() {
            return self.archive.is_none()
                && archive.is_none()
                && self
                    .network
                    .as_ref()
                    .zip(network)
                    .is_some_and(|(left, right)| {
                        left.provider == right.provider && left.opaque == right.opaque
                    });
        }
        match (self.archive.as_ref(), archive) {
            (None, None) => self.path == path,
            // Archives inside the archive count: a folder of an archive inside an archive
            // is not the folder of the enclosing archive that holds it.
            (Some(left), Some(right)) => left.same_location(right),
            _ => false,
        }
    }

    pub fn same_location(&self, other: &Self) -> bool {
        self.matches_location(&other.path, other.archive.as_ref(), other.network.as_ref())
    }

    /// Presentation only, never parsed back: an archive location reads as the path bar
    /// shows it, the archive file followed by its folders and the archives inside it,
    /// for example `D:\data\a.zip\docs\b.7z\inner`.
    pub fn display_text(&self) -> String {
        if let Some(network) = &self.network {
            return network.display.clone();
        }
        let Some(archive) = &self.archive else {
            return self.path.to_string_lossy().into_owned();
        };
        crate::ArchiveLocation::unavailable(archive.clone()).display_text()
    }
}
