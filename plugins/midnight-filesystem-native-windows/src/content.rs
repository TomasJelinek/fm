use crate::system::{self, Snapshot};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeReader};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::{FileExt, OpenOptionsExt},
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
fn open(path: &Path, access: Access) -> Result<File, Failure> {
    let mut options = OpenOptions::new();
    match access {
        Access::Content => {
            #[cfg(test)]
            DATA_OPENS.with(|opens| opens.set(opens.get() + 1));
            options.read(true);
        }
        // Attribute-only opens request no data access: no share-mode conflict,
        // no SMB data open or oplock break and no on-access content scan.
        Access::Attributes => {
            options.access_mode(FILE_READ_ATTRIBUTES);
        }
    }
    let file = options
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::path::literal(path)?)
        .map_err(Failure::from_io)?;
    system::require_readable_file(&file)?;
    Ok(file)
}
#[derive(Clone, Copy)]
enum Access {
    Content,
    Attributes,
}
#[cfg(test)]
thread_local! {
    /// Content opens on this thread; validation must not add any.
    static DATA_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self, Failure> {
        let file = open(path, Access::Content)?;
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
        drop(file);
        // Only Reader::open opens content. The current name is checked through an
        // attribute-only handle with the same type, tag, identity and revision checks.
        let current = open(&self.path, Access::Attributes).map_err(|_| changed())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Client, Temp};

    #[test]
    fn validation_checks_the_current_name_without_opening_content_again() {
        let temp = Temp::new();
        let path = temp.path.join("selected");
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        let client = Client::new();
        let (mount, node) = client.resolve(&path);
        let opens = || DATA_OPENS.with(std::cell::Cell::get);
        let before = opens();
        let Response::Opened { reader, .. } = client.call(Request::OpenRead {
            mount,
            node: node.id,
        }) else {
            panic!("regular file must open")
        };
        for _ in 0..3 {
            assert_eq!(
                client.call(Request::ValidateRead { reader }),
                Response::Unit
            );
        }
        for offset in [0, 2048] {
            assert_eq!(
                client.call(Request::ReadAt {
                    reader,
                    offset,
                    length: 2048,
                }),
                Response::Content {
                    bytes: vec![7u8; 2048],
                    eof: offset == 2048,
                }
            );
        }
        assert_eq!(opens() - before, 1, "only OpenRead opens file content");
        // The attribute-only check still detects a replaced name.
        std::fs::rename(&path, temp.path.join("held")).unwrap();
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        assert!(matches!(
            client.call(Request::ValidateRead { reader }),
            Response::Failed(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ));
        assert_eq!(opens() - before, 1);
        assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
        assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    }
}
