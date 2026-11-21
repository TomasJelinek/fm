//! Linux and macOS files are shown with the icon of their type from the desktop's
//! icon theme; no listed file's own content is read for artwork.
use std::ffi::OsStr;

pub(super) fn carries_own_artwork(_: &OsStr) -> bool {
    false
}
