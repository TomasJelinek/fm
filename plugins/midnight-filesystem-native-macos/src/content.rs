use crate::system;
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeReader};
use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    device: i32,
    inode: u64,
    generation: u32,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    created: (i64, i64),
}
impl Snapshot {
    fn from_stat(value: &libc::stat) -> Result<Self, Failure> {
        if value.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(changed());
        }
        Ok(Self {
            device: value.st_dev,
            inode: value.st_ino,
            generation: value.st_gen,
            length: u64::try_from(value.st_size).map_err(|_| changed())?,
            modified: (value.st_mtime, value.st_mtime_nsec),
            changed: (value.st_ctime, value.st_ctime_nsec),
            created: (value.st_birthtime, value.st_birthtime_nsec),
        })
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"native.macos.read.v1".to_vec();
        bytes.extend(self.device.to_le_bytes());
        bytes.extend(self.inode.to_le_bytes());
        bytes.extend(self.generation.to_le_bytes());
        bytes.extend(self.length.to_le_bytes());
        for (seconds, nanos) in [self.modified, self.changed, self.created] {
            bytes.extend(seconds.to_le_bytes());
            bytes.extend(nanos.to_le_bytes());
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
impl Reader {
    pub(crate) fn open(path: &Path, context: &CallContext<'_>) -> Result<Self, Failure> {
        let file = system::open_regular(path, context)?;
        let snapshot = Snapshot::from_stat(&system::stat_file(&file)?)?;
        let reader = Self {
            file,
            path: path.to_owned(),
            revision: snapshot.bytes(),
            snapshot,
        };
        reader.validate(context)?;
        Ok(reader)
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
        if Snapshot::from_stat(&system::stat_file(&self.file)?)? != self.snapshot {
            return Err(changed());
        }
        let current = system::stat_path(&self.path, context).map_err(|error| {
            if error.code == ErrorCode::Cancelled {
                error
            } else {
                changed()
            }
        })?;
        if Snapshot::from_stat(&current)? != self.snapshot {
            return Err(changed());
        }
        system::cancel(context)
    }
    fn read_at(
        &self,
        offset: u64,
        length: u32,
        context: &CallContext<'_>,
    ) -> Result<Vec<u8>, Failure> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length as usize).map_err(|_| {
            Failure::new(ErrorCode::ResourceLimit, "Read allocation budget exceeded.")
        })?;
        bytes.resize(length as usize, 0);
        let mut done = 0;
        while done < bytes.len() {
            system::cancel(context)?;
            let position = offset.checked_add(done as u64).ok_or_else(changed)?;
            match self.file.read_at(&mut bytes[done..], position) {
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
