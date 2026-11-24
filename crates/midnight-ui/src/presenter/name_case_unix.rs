//! Linux folders name their entries exactly as they were created: a name in other letter
//! case names another entry or none.
use midnight_core::Entry;
use std::ffi::OsStr;

/// No entry is named in other letter case.
pub(super) fn case_variant<'a>(_: &'a [Entry], _: &OsStr) -> Option<&'a Entry> {
    None
}

/// No file is named in other letter case.
pub(super) fn lists_file_in_other_case(_: &[Entry], _: &OsStr) -> bool {
    false
}
