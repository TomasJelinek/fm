//! macOS loader/signing and stable native identity are deliberately deferred.
use crate::{Error, api::abi};
use std::{fs::File, io, path::Path};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub length: u64,
}
pub(crate) fn open_read(_: &Path) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}
pub(crate) fn snapshot(_: &File) -> io::Result<Snapshot> {
    Err(io::ErrorKind::Unsupported.into())
}
pub(crate) struct Library;
impl Library {
    pub(crate) fn open(_: &Path) -> Result<Self, Error> {
        Err(Error(
            "Native viewer plugins unsupported on this platform".into(),
        ))
    }
    pub(crate) fn entry(&self) -> Result<abi::Entry, Error> {
        Err(Error(
            "Native viewer plugins unsupported on this platform".into(),
        ))
    }
}
