use crate::system::{self, Snapshot};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeReader};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows_sys::Win32::Storage::FileSystem::*;

pub struct Reader {
    path: PathBuf,
    file: Mutex<File>,
    snapshot: Snapshot,
    revision: Vec<u8>,
}
fn open(path: &Path) -> Result<File, Failure> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::path::literal(path)?)
        .map_err(Failure::from_io)?;
    let metadata = file.metadata().map_err(Failure::from_io)?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Read requires a regular non-reparse file.",
        ));
    }
    Ok(file)
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self, Failure> {
        let file = open(path)?;
        let snapshot = system::snapshot(&file)?;
        let revision = snapshot.bytes();
        Ok(Self {
            path: path.to_owned(),
            file: Mutex::new(file),
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
        if context.is_cancelled() {
            return Err(Failure::new(ErrorCode::Cancelled, "Read canceled."));
        }
        let file = self.file.lock().unwrap_or_else(|error| error.into_inner());
        if system::snapshot(&file)? != self.snapshot {
            return Err(changed());
        }
        let current = open(&self.path).map_err(|_| changed())?;
        if system::snapshot(&current)? != self.snapshot {
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
        if context.is_cancelled() {
            return Err(Failure::new(ErrorCode::Cancelled, "Read canceled."));
        }
        let file = self.file.lock().unwrap_or_else(|error| error.into_inner());
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|_| Failure::new(ErrorCode::ResourceLimit, "Read allocation budget."))?;
        bytes.resize(length as usize, 0);
        let mut done = 0;
        while done < bytes.len() {
            if context.is_cancelled() {
                return Err(Failure::new(ErrorCode::Cancelled, "Read canceled."));
            }
            match file.seek_read(&mut bytes[done..], offset + done as u64) {
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
