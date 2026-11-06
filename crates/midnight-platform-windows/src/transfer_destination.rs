use super::*;
use midnight_core::{StagedTransferFile, TransferPublication};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::PathBuf,
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
};

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    path: PathBuf,
    identity: TransferIdentity,
}
impl Directory {
    pub(super) fn from_file(file: File, path: PathBuf) -> Result<Self, FsError> {
        if !file.metadata().map_err(|e| error(&path, e))?.is_dir() {
            return Err(invalid(&path, "destination is not a directory"));
        }
        Ok(Self {
            identity: identity(&file, &path)?,
            file: Arc::new(file),
            path,
        })
    }
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        self.identity
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        let visible = native::open_parent(&self.path).map_err(|e| error(&self.path, e))?;
        if identity(&visible, &self.path)? != self.identity {
            return Err(invalid(&self.path, "destination directory changed"));
        }
        Ok(())
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.path.join(name);
        match native::open_relative(
            &self.file,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        ) {
            Ok(file) => entry(&file, &path).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(&path, e)),
        }
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        // Resolve the retained handle, then inspect native identities of its actual ancestors.
        // There is no lexical case folding or assumption that drive letters are identities.
        let actual = final_path(&self.file).map_err(|e| error(&self.path, e))?;
        for ancestor in actual.ancestors().filter(|p| p.has_root()) {
            checked(cancel, &self.path)?;
            let file = native::open_parent(ancestor).map_err(|e| error(&self.path, e))?;
            if identity(&file, ancestor)? == source {
                return Err(invalid(
                    &self.path,
                    "destination is inside the source directory",
                ));
            }
        }
        self.revalidate(cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.path.join(name);
        let file = native::open_relative(
            &self.file,
            name,
            FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE,
        )
        .map_err(|e| error(&path, e))?;
        Self::from_file(file, path)
            .map(|directory| Arc::new(directory) as Arc<dyn TransferDirectory>)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.revalidate(cancel)?;
        for _ in 0..16 {
            checked(cancel, &self.path)?;
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|e| error(&self.path, io::Error::other(e.to_string())))?;
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let name = OsString::from(format!(".midnight-copy-{suffix}"));
            let path = self.path.join(&name);
            match native::open_relative(
                &self.file,
                &name,
                FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
                FILE_SHARE_READ,
                FILE_CREATE,
                FILE_NON_DIRECTORY_FILE,
            ) {
                Ok(file) => {
                    return Ok(Box::new(Stage {
                        file,
                        directory: self.clone(),
                        published: false,
                        cleaned: false,
                        path,
                    }));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(&path, e)),
            }
        }
        Err(error(
            &self.path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "exclusive staging names collided repeatedly",
            ),
        ))
    }
}

struct Stage {
    file: File,
    directory: Directory,
    published: bool,
    cleaned: bool,
    path: PathBuf,
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.file.read(b)
    }
}
impl Write for Stage {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.file.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.file.seek(p)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        self.file.sync_all().map_err(|e| error(&self.path, e))
    }
    fn len(&self) -> Result<u64, FsError> {
        self.file
            .metadata()
            .map(|m| m.len())
            .map_err(|e| error(&self.path, e))
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published || self.cleaned {
            return Err(invalid(&self.path, "stage is no longer available"));
        }
        self.directory.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.directory.path.join(name);
        checked(cancel, &path)?;
        native::rename(&self.file, name).map_err(|e| error(&path, e))?;
        self.published = true;
        // Windows has no general directory-entry flush contract for this API. The
        // staged file was synced before verification; no parent power-loss guarantee is made.
        Ok(TransferPublication::Published(path))
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        if !self.published && !self.cleaned {
            // Disposition is bound to our retained inode, never to a pathname that
            // another process could replace. Denied delete sharing protects this handle.
            native::remove(&self.file).map_err(|e| error(&self.path, e))?;
            self.cleaned = true;
        }
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn final_path(file: &File) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 256];
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "native path length is unrepresentable",
            )
        })?;
        // SAFETY: writable UTF-16 buffer and live handle; length result checked before use.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                VOLUME_NAME_DOS,
            )
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= capacity {
            buffer.resize(count as usize + 1, 0);
            continue;
        }
        buffer.truncate(count as usize);
        return Ok(PathBuf::from(OsString::from_wide(&buffer)));
    }
}
