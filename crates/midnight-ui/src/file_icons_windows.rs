//! Windows programs carry their own icon resources; the Shell shows them instead of
//! the generic program icon of their type.
use std::ffi::OsStr;

pub(super) fn carries_own_artwork(extension: &OsStr) -> bool {
    extension.eq_ignore_ascii_case("exe")
}
