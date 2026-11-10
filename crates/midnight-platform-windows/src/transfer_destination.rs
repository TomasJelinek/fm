use super::*;
use midnight_core::{
    StagedTransferFile, TransferCapabilities, TransferPublication, TransferReplacementPolicy,
    TransferTarget,
};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::PathBuf,
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
};

#[path = "transfer_preservation.rs"]
mod preservation;
#[path = "transfer_recovery.rs"]
pub(super) mod recovery;

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    path: PathBuf,
    identity: TransferIdentity,
}
impl Directory {
    pub(in crate::transfer) fn native_file(&self) -> &File {
        &self.file
    }
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
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        super::source::Source::open(self.file.clone(), self.path.clone(), name, cancel)
            .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        self.identity
    }
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities {
            directory_merge: true,
            conditional_overwrite: false,
            recoverable_replacement: recovery::supported(self),
        }
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        source.device == self.identity.device && recovery::supported(self)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        recovery::prepare_target(self, name, cancel)
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        recovery::open_merge(self, name, expected, cancel)
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let mut stage = self.stage_file(cancel)?;
        initialize_stage(stage.as_mut(), |stage| {
            stage.prepare_replacement(name, expected, policy, cancel)
        })?;
        Ok(stage)
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let preserved = preservation::Snapshot::capture(self, name, expected.as_ref(), cancel)?;
        let mut stage = self.create_stage(false, Some(preserved), cancel)?;
        initialize_stage(&mut stage, |stage| {
            stage
                .preserved
                .as_ref()
                .ok_or_else(|| invalid(&stage.path, "missing metadata"))?
                .initialize(&stage.file, &stage.path)?;
            expected.revalidate(cancel)?;
            stage.prepare_replacement(name, expected, policy, cancel)
        })?;
        Ok(Box::new(stage))
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
        self.make_stage(false, cancel)
    }
    fn stage_file_with_metadata(
        &self,
        policy: midnight_core::TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.make_stage(
            policy == midnight_core::TransferMetadataPolicy::RequireModifiedTime,
            cancel,
        )
    }
}
impl Directory {
    fn make_stage(
        &self,
        metadata: bool,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.create_stage(metadata, None, cancel)
            .map(|stage| Box::new(stage) as Box<dyn StagedTransferFile>)
    }
    fn create_stage(
        &self,
        metadata: bool,
        preserved: Option<preservation::Snapshot>,
        cancel: &Cancellation,
    ) -> Result<Stage, FsError> {
        self.create_stage_named(metadata, preserved, cancel, || {
            let mut random = [0u8; 8];
            getrandom::fill(&mut random)
                .map_err(|e| error(&self.path, io::Error::other(e.to_string())))?;
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Ok(OsString::from(format!(".me-{suffix}")))
        })
    }

    fn create_stage_named(
        &self,
        metadata: bool,
        preserved: Option<preservation::Snapshot>,
        cancel: &Cancellation,
        mut next_name: impl FnMut() -> Result<OsString, FsError>,
    ) -> Result<Stage, FsError> {
        self.revalidate(cancel)?;
        for _ in 0..16 {
            checked(cancel, &self.path)?;
            let name = next_name()?;
            let path = self.path.join(&name);
            let created = if preserved.is_some() {
                recovery::create_private_file(&self.file, &name)
            } else {
                native::open_relative(
                    &self.file,
                    &name,
                    FILE_READ_DATA
                        | FILE_WRITE_DATA
                        | FILE_READ_ATTRIBUTES
                        | DELETE
                        | SYNCHRONIZE
                        | if metadata { FILE_WRITE_ATTRIBUTES } else { 0 },
                    FILE_SHARE_READ,
                    FILE_CREATE,
                    FILE_NON_DIRECTORY_FILE,
                )
            };
            match created {
                Ok(file) => {
                    return Ok(Stage {
                        file,
                        directory: self.clone(),
                        published: false,
                        published_name: None,
                        cleaned: false,
                        path,
                        replacement: None,
                        metadata,
                        preserved,
                    });
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

#[cfg(test)]
#[path = "transfer_stage_tests.rs"]
mod stage_tests;

struct Stage {
    metadata: bool,
    preserved: Option<preservation::Snapshot>,
    file: File,
    directory: Directory,
    published: bool,
    published_name: Option<OsString>,
    cleaned: bool,
    path: PathBuf,
    replacement: Option<recovery::Replacement>,
}

/// Construction failures still own an unpublished stage. Preserve both the primary
/// error and any exact-path cleanup refusal; Drop is only a final best-effort retry.
fn initialize_stage<T: StagedTransferFile + ?Sized>(
    stage: &mut T,
    initialize: impl FnOnce(&mut T) -> Result<(), FsError>,
) -> Result<(), FsError> {
    initialize(stage).map_err(|failure| {
        if let Err(mut cleanup) = stage.cleanup() {
            // Preserve the native cleanup kind and residual path: an Interrupted
            // setup error must not hide this refusal as ordinary cancellation.
            cleanup
                .message
                .push_str(&format!("; stage initialization failed: {failure}"));
            return cleanup;
        }
        failure
    })
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
    fn set_modified_time(
        &mut self,
        expected: std::time::SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if self.published || self.cleaned || !self.metadata {
            return Err(unsupported(
                &self.path,
                "modified time can only be set on an owned unpublished metadata stage",
            ));
        }
        super::metadata::set(&self.file, &self.path, expected, cancel)
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        if self.cleaned || !self.metadata {
            return Err(unsupported(
                &self.path,
                "required modified time is unsupported by this stage",
            ));
        }
        super::metadata::modified(&self.file, &self.path, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        if self.cleaned {
            return Err(invalid(&self.path, "stage was cleaned up"));
        }
        super::validation::rewind(&self.file, &self.path, cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        let name = self
            .published_name
            .as_ref()
            .ok_or_else(|| invalid(&self.path, "stage has no successful publication"))?;
        let own = identity(&self.file, &self.path)?;
        let visible = self
            .directory
            .inspect(name, cancel)?
            .ok_or_else(|| invalid(&self.path, "published target disappeared"))?;
        if visible.item.kind != EntryKind::File || visible.identity != own {
            return Err(invalid(&self.path, "published target name changed"));
        }
        Ok(own)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.bind_replacement(name, expected, policy, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if let Some(snapshot) = &self.preserved {
            snapshot.verify(&self.file, &self.path)?;
        }
        self.replace(name, cancel, || {})
    }
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if let Some(snapshot) = &self.preserved {
            snapshot.apply_attributes(&self.file, &self.path)?;
            snapshot.verify(&self.file, &self.path)?;
        }
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
        self.published_name = Some(name.to_owned());
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
