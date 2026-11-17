use super::*;
use crate::directory_records::Cursor;
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    io::{Read, Seek, SeekFrom},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
};

pub(super) struct Source {
    entry: TransferEntry,
    native: Native,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    readable: Option<File>,
    stamp: (u64, u64, u64),
    names: VecDeque<(OsString, Native)>,
    cursor: Cursor,
    move_enabled: bool,
    // Last: all native fields, including lazy readable upgrades, close before this signal.
    _namespace_owner: super::namespace::NativeOwner,
}
/// A source's directory and its already proven identity, for opening its children.
struct Parent {
    file: Arc<File>,
    path: PathBuf,
    identity: Option<TransferIdentity>,
}
impl Source {
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            Parent {
                file: parent,
                path: parent_path,
                identity: None,
            },
            name,
            None,
            false,
            cancel,
        )
    }
    pub(super) fn open_for_move(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            Parent {
                file: parent,
                path: parent_path,
                identity: None,
            },
            name,
            None,
            true,
            cancel,
        )
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    /// `listed` is the child's classification from its directory record. Without it, a
    /// no-follow probe classifies the entry first.
    fn open_with_access(
        parent: Parent,
        name: &OsStr,
        listed: Option<Native>,
        move_root: bool,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let path = parent.path.join(name);
        checked(cancel, &path)?;
        validate_name(name).map_err(|e| error(&path, e))?;
        let (expected, probed) = match listed {
            Some(native) => (native, None),
            None => {
                let probe = native::open_relative(
                    &parent.file,
                    name,
                    FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    FILE_OPEN,
                    0,
                )
                .map_err(|e| error(&path, e))?;
                let native = Native::query(&probe).map_err(|e| error(&path, e))?;
                (native, Some(identity(&probe, &path)?))
            }
        };
        let options = match expected.kind() {
            EntryKind::Directory => FILE_DIRECTORY_FILE,
            EntryKind::File => FILE_NON_DIRECTORY_FILE,
            _ => {
                return Err(unsupported(
                    &path,
                    "copying links and special entries is not supported",
                ));
            }
        };
        // Denying write and delete sharing makes ordinary concurrent writers/renamers
        // fail rather than produce a mixed stream. Existing incompatible handles fail here.
        let access = FILE_READ_ATTRIBUTES
            | SYNCHRONIZE
            | if move_root { DELETE } else { FILE_READ_DATA }
            | if expected.directory() {
                FILE_TRAVERSE
            } else {
                0
            };
        let file = native::open_relative(
            &parent.file,
            name,
            access,
            FILE_SHARE_READ,
            FILE_OPEN,
            options,
        )
        .map_err(|e| error(&path, e))?;
        let (entry, native, meta) = classified_with_metadata(&file, &path)?;
        if probed.is_some_and(|probed| probed != entry.identity) || native.kind() != expected.kind()
        {
            return Err(invalid(&path, "source changed while opening"));
        }
        let stamp = (
            meta.file_size(),
            meta.last_write_time(),
            meta.creation_time(),
        );
        // A cloud placeholder or deduplicated/WOF file is read through its filter so its
        // data hydrates instead of copying (and then verifying) the raw reparse stub.
        let readable = if !move_root && native.filtered() && !native.directory() {
            let content = crate::reparse::reopen_content(
                &file,
                native,
                FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ,
            )
            .map_err(|e| error(&path, e))?;
            if identity(&content, &path)? != entry.identity {
                return Err(invalid(&path, "source changed while opening its content"));
            }
            Some(content)
        } else {
            None
        };
        let parent_identity = match parent.identity {
            Some(identity) => identity,
            None => identity(&parent.file, &parent.path)?,
        };
        let namespace_owner = super::namespace::owner(&[parent_identity, entry.identity]);
        Ok(Self {
            entry,
            native,
            parent: parent.file,
            parent_path: parent.path,
            parent_identity,
            name: name.to_owned(),
            file: Arc::new(file),
            readable,
            stamp,
            names: VecDeque::new(),
            cursor: Cursor::default(),
            move_enabled: move_root,
            _namespace_owner: namespace_owner,
        })
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        self.ensure_readable()?;
        let names = &mut self.names;
        let directory = self.readable.as_ref().unwrap_or(&self.file);
        self.cursor
            .next_batch(directory, &mut |record| {
                names.push_back((OsString::from_wide(record.name), record.native));
                Ok(())
            })
            .map_err(|e| error(&self.entry.item.path, e))
    }
    /// Opens a listed child relative to this directory, reusing its proven identity.
    fn open_listed(
        &self,
        name: &OsStr,
        native: Native,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            Parent {
                file: self.file.clone(),
                path: self.entry.item.path.clone(),
                identity: Some(self.entry.identity),
            },
            name,
            Some(native),
            false,
            cancel,
        )
    }
}
impl TransferSource for Source {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        super::namespace::source_pin(
            &self.parent,
            &self.parent_path,
            &self.name,
            &self.entry,
            cancel,
        )
        .map(Some)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        crate::mutation::discovery::walk(
            &self.parent,
            &self.file,
            &self.name,
            &self.entry.item.path,
            cancel,
            visit,
        )?;
        self.revalidate(cancel)
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(
                &self.entry.item.path,
                "named child requires a directory",
            ));
        }
        Self::open(
            self.file.clone(),
            self.entry.item.path.clone(),
            name,
            cancel,
        )
        .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMove>, FsError> {
        if !self.move_enabled {
            return Err(unsupported(
                &self.entry.item.path,
                "Copy sources have no Move authority",
            ));
        }
        self.next_name(cancel)?
            .map(|(name, _)| {
                super::move_source::prepare_child(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    cancel,
                )
            })
            .transpose()
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferModifiedTimePin>, FsError> {
        if self.entry.item.kind != EntryKind::File {
            return Err(unsupported(
                &self.entry.item.path,
                "required modified time supports regular files only",
            ));
        }
        self.revalidate(cancel)?;
        super::metadata::prepare(
            self.file.clone(),
            self._namespace_owner.clone(),
            &self.entry.item.path,
            cancel,
        )
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        if self.entry.item.kind != EntryKind::File {
            return Err(unsupported(
                &self.entry.item.path,
                "only regular files support retained validation",
            ));
        }
        checked(cancel, &self.entry.item.path)?;
        self.ensure_readable()?;
        super::validation::rewind(self.read_file(), &self.entry.item.path, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        checked(cancel, &self.entry.item.path)?;
        self.ensure_readable()?;
        let file = self.read_file();
        let length = file
            .metadata()
            .map_err(|e| error(&self.entry.item.path, e))?
            .len();
        if offset > length {
            return Err(invalid(
                &self.entry.item.path,
                "validation offset exceeds retained file length",
            ));
        }
        let reached = (&*file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(&self.entry.item.path, e))?;
        if reached != offset {
            return Err(invalid(
                &self.entry.item.path,
                "validation seek did not reach requested offset",
            ));
        }
        Ok(length)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        let visible = native::open_parent(&self.parent_path).map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.parent_identity {
            return Err(invalid(path, "source parent changed"));
        }
        let visible = native::open_relative(
            &self.parent,
            &self.name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.entry.identity {
            return Err(invalid(path, "source identity changed"));
        }
        let meta = self.file.metadata().map_err(|e| error(path, e))?;
        if self.entry.item.kind == EntryKind::File
            && (
                meta.file_size(),
                meta.last_write_time(),
                meta.creation_time(),
            ) != self.stamp
        {
            return Err(invalid(path, "source content changed"));
        }
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        checked(cancel, &self.entry.item.path)?;
        if self.entry.item.kind != EntryKind::File {
            return Err(invalid(
                &self.entry.item.path,
                "directory content is not a byte stream",
            ));
        }
        self.ensure_readable()?;
        self.read_file()
            .read(buffer)
            .map_err(|e| error(&self.entry.item.path, e))
    }
    fn supports_read_retry(&self) -> bool {
        self.entry.item.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if !self.supports_read_retry()
            || self
                .entry
                .item
                .metadata
                .size
                .is_none_or(|size| offset > size)
        {
            return Err(invalid(
                &self.entry.item.path,
                "invalid source retry offset",
            ));
        }
        self.ensure_readable()?;
        self.read_file()
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(&self.entry.item.path, e))?;
        self.revalidate(cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.next_name(cancel)?
            .map(|(name, native)| {
                self.open_listed(&name, native, cancel)
                    .map(|source| Box::new(source) as Box<dyn TransferSource>)
            })
            .transpose()
    }
}
impl Source {
    fn read_file(&self) -> &File {
        self.readable.as_ref().unwrap_or(&self.file)
    }

    fn ensure_readable(&mut self) -> Result<(), FsError> {
        if !self.move_enabled || self.readable.is_some() {
            return Ok(());
        }
        if self.entry.item.kind == EntryKind::Directory {
            let held = native::open_relative(
                &self.parent,
                &self.name,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE,
            )
            .map_err(|e| error(&self.entry.item.path, e))?;
            if identity(&held, &self.entry.item.path)? != self.entry.identity {
                return Err(invalid(
                    &self.entry.item.path,
                    "directory changed before listing upgrade",
                ));
            }
            self.readable = Some(held);
            return Ok(());
        }
        // Upgrade the retained object only when copying or enumerating a merge.
        // The original DELETE pin excludes writers/renamers; this read handle
        // permits that existing DELETE access without requesting it again. A cloud
        // placeholder or deduplicated file is read through its filter.
        let readable = crate::reparse::reopen_content(
            &self.file,
            self.native,
            FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
        )
        .map_err(|e| error(&self.entry.item.path, e))?;
        if identity(&readable, &self.entry.item.path)? != self.entry.identity {
            return Err(invalid(
                &self.entry.item.path,
                "source changed before read upgrade",
            ));
        }
        self.readable = Some(readable);
        Ok(())
    }

    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<(OsString, Native)>, FsError> {
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        loop {
            if let Some(next) = self.names.pop_front() {
                return Ok(Some(next));
            }
            // The directory's path binding is revalidated once per native batch and once
            // at its end. Each child is opened relative to the retained handle and the
            // engine revalidates every child's own parent binding before using it.
            self.revalidate(cancel)?;
            if self.cursor.ended() {
                return Ok(None);
            }
            self.fill_names()?;
        }
    }
}
