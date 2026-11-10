//! Lossless native and archive Hot Paths; display text is never a locator.
use crate::ArchiveAddress;
use std::path::{Path, PathBuf};

/// Stable callback identity, independent of names and displayed paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BookmarkId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub id: BookmarkId,
    /// Retained for compatibility; the manager presents the location instead.
    pub name: String,
    /// Native folder, or the archive container's native parent as a return path.
    pub path: PathBuf,
    pub archive: Option<ArchiveAddress>,
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
    pub fn matches_location(&self, path: &Path, archive: Option<&ArchiveAddress>) -> bool {
        match (self.archive.as_ref(), archive) {
            (None, None) => self.path == path,
            (Some(left), Some(right)) => {
                left.provider == right.provider
                    && left.container == right.container
                    && left
                        .format
                        .or_else(|| crate::archive_format(&left.container))
                        == right
                            .format
                            .or_else(|| crate::archive_format(&right.container))
                    && left.components == right.components
            }
            _ => false,
        }
    }

    pub fn same_location(&self, other: &Self) -> bool {
        self.matches_location(&other.path, other.archive.as_ref())
    }

    /// Presentation only. Literal `@` in a native name is never parsed back.
    pub fn display_text(&self) -> String {
        let Some(archive) = &self.archive else {
            return self.path.to_string_lossy().into_owned();
        };
        let mut text = archive.container.to_string_lossy().into_owned();
        text.push('@');
        for (index, component) in archive.components.iter().enumerate() {
            if index > 0 {
                text.push(std::path::MAIN_SEPARATOR);
            }
            text.push_str(&String::from_utf8_lossy(component));
        }
        text
    }
}
