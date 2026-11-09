use crate::system;
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeReader};
use std::{
    fs::File,
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    device: u64,
    inode: u64,
    length: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
impl Snapshot {
    fn of(file: &File) -> Result<Self, Failure> {
        let value = file.metadata().map_err(Failure::from_io)?;
        Ok(Self {
            device: value.dev(),
            inode: value.ino(),
            length: value.len(),
            mtime: value.mtime(),
            mtime_ns: value.mtime_nsec(),
            ctime: value.ctime(),
            ctime_ns: value.ctime_nsec(),
        })
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"native.linux.read.v1".to_vec();
        for value in [self.device, self.inode, self.length] {
            bytes.extend(value.to_le_bytes())
        }
        for value in [self.mtime, self.mtime_ns, self.ctime, self.ctime_ns] {
            bytes.extend(value.to_le_bytes())
        }
        bytes
    }
}
pub struct Reader {
    file: File,
    path: PathBuf,
    snapshot: Snapshot,
    revision: Vec<u8>,
}
fn open(path: &Path, context: &CallContext<'_>) -> Result<File, Failure> {
    open_with_hook(path, context, || {})
}
fn open_with_hook(
    path: &Path,
    context: &CallContext<'_>,
    before_data_open: impl FnOnce(),
) -> Result<File, Failure> {
    let file = system::open_path(path, libc::O_PATH | libc::O_NOFOLLOW, context)?;
    if !file.metadata().map_err(Failure::from_io)?.is_file() {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Read requires a regular non-link file.",
        ));
    }
    let filesystem = system::filesystem(&file)?;
    if matches!(
        filesystem.f_type as u64,
        0x9fa0
            | 0x62656572
            | 0x64626720
            | 0x74726163
            | 0x73636673
            | 0x62656570
            | 0x27e0eb
            | 0x63677270
    ) {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Pseudo-filesystem content has no supported known-length ReadAt contract.",
        ));
    }
    // Qualification precedes every content open. A later path substitution can
    // invalidate provenance, but cannot make us open a device or FIFO for data.
    before_data_open();
    system::reopen_regular(&file)
}
#[cfg(test)]
#[allow(unsafe_code)]
#[path = "content_tests.rs"]
mod tests;
impl Reader {
    pub fn open(path: &Path, context: &CallContext<'_>) -> Result<Self, Failure> {
        let file = open(path, context)?;
        let snapshot = Snapshot::of(&file)?;
        let revision = snapshot.bytes();
        Ok(Self {
            file,
            path: path.to_owned(),
            snapshot,
            revision,
        })
    }
}
impl NativeReader for Reader {
    fn length(&self) -> u64 {
        self.snapshot.length
    }
    fn revision(&self) -> &[u8] {
        &self.revision
    }
    fn validate(&self, context: &CallContext<'_>) -> Result<(), Failure> {
        system::cancel(context)?;
        if Snapshot::of(&self.file)? != self.snapshot {
            return Err(changed());
        }
        let current = open(&self.path, context).map_err(|error| {
            if error.code == ErrorCode::Cancelled {
                error
            } else {
                changed()
            }
        })?;
        if Snapshot::of(&current)? != self.snapshot {
            return Err(changed());
        }
        Ok(())
    }
    fn read_at(
        &self,
        offset: u64,
        length: u32,
        context: &CallContext<'_>,
    ) -> Result<Vec<u8>, Failure> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|_| Failure::new(ErrorCode::ResourceLimit, "Read allocation budget."))?;
        bytes.resize(length as usize, 0);
        let mut done = 0;
        while done < bytes.len() {
            system::cancel(context)?;
            match self.file.read_at(&mut bytes[done..], offset + done as u64) {
                Ok(0) => return Err(changed()),
                Ok(count) => done += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Failure::from_io(error)),
            }
        }
        Ok(bytes)
    }
}
fn changed() -> Failure {
    Failure::new(ErrorCode::SourceChanged, "Captured native content changed.")
}
