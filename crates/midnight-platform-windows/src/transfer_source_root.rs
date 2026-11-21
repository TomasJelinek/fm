//! A selected Copy item, prepared before its job is admitted and opened when first used.
//!
//! The engine prepares a Copy job's selected items to plan its namespace claims before
//! admission grants the job, and keeps them while the job waits for granted jobs that may
//! still change those items and the folders they are in. Windows refuses a rename into a
//! folder (another job's publication of a copied file, a program saving by rename) while
//! any handle on that folder denies write sharing, a rename or removal of an entry while a
//! handle denies delete sharing, and a write while a handle denies write sharing; the
//! source that reads an item holds such a guard (`open_ordinary`). A prepared item
//! therefore holds no handle of its own: only its folder's attribute handle, which shares
//! everything, and the identity, kind, metadata and revision it observed through a handle
//! that takes no part in sharing checks. The first listing, discovery, read or
//! modified-time pin opens the guarded source on the same name in that folder, refuses any
//! other object and, for a file, any other revision, and keeps it for the rest of the job.
//! A link keeps its own guarded capture from the start.
use super::*;
use std::sync::Mutex;

/// Size, LastWriteTime and creation time, compared as `Source` compares them.
type Stamp = (u64, u64, u64);

/// Prepares the selected item `name` of `parent` for Copy (see the module documentation).
pub(in crate::transfer) fn prepare(
    parent: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    links: CopyLinkPolicy,
    cancel: &Cancellation,
) -> Result<Box<dyn TransferSource>, FsError> {
    let path = parent_path.join(name);
    checked(cancel, &path)?;
    validate_name(name).map_err(|e| error(&path, e))?;
    let (entry, stamp) = observe(&parent, name, &path)?;
    if !matches!(entry.item.kind, EntryKind::File | EntryKind::Directory) {
        // A link is projected, or a link or special entry refused, by the source itself.
        return Source::open_with_links(parent, parent_path, name, links, cancel)
            .map(|source| Box::new(source) as Box<dyn TransferSource>);
    }
    let parent_identity = identity(&parent, &parent_path)?;
    let namespace_owner = super::super::namespace::owner(&[parent_identity, entry.identity]);
    Ok(Box::new(Prepared {
        entry,
        parent,
        parent_path,
        parent_identity,
        name: name.to_owned(),
        links,
        stamp,
        active: Mutex::new(None),
        _namespace_owner: namespace_owner,
    }))
}

/// The entry and revision of `name`, read through an attribute-only handle (it takes no
/// part in sharing checks) that is closed again before this returns.
fn observe(parent: &File, name: &OsStr, path: &Path) -> Result<(TransferEntry, Stamp), FsError> {
    let file = native::open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        0,
    )
    .map_err(|e| error(path, e))?;
    let (entry, _, metadata) = classified_with_metadata(&file, path)?;
    let stamp = (
        metadata.file_size(),
        metadata.last_write_time(),
        metadata.creation_time(),
    );
    Ok((entry, stamp))
}

struct Prepared {
    entry: TransferEntry,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    links: CopyLinkPolicy,
    stamp: Stamp,
    /// The guarded source, from the first use on.
    active: Mutex<Option<Source>>,
    // Last: the parent handle and the guarded source close before this signal.
    _namespace_owner: super::super::namespace::NativeOwner,
}

impl Prepared {
    fn path(&self) -> &Path {
        &self.entry.item.path
    }

    fn unavailable(&self) -> FsError {
        invalid(
            self.path(),
            "prepared Copy source is unavailable after a worker failure",
        )
    }

    /// The binding and revision observed at preparation, checked without a guard: the
    /// folder's path still leads to the same folder, and the name to the same object.
    fn revalidate_prepared(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = self.path();
        checked(cancel, path)?;
        revalidate_parent(&self.parent_path, self.parent_identity, None, path, cancel)?;
        let (entry, stamp) = observe(&self.parent, &self.name, path)?;
        self.check(entry.identity, entry.item.kind, stamp)
    }

    fn check(
        &self,
        identity: TransferIdentity,
        kind: EntryKind,
        stamp: Stamp,
    ) -> Result<(), FsError> {
        if identity != self.entry.identity || kind != self.entry.item.kind {
            return Err(invalid(self.path(), "source identity changed"));
        }
        if kind == EntryKind::File && stamp != self.stamp {
            return Err(invalid(self.path(), "source content changed"));
        }
        Ok(())
    }

    /// Opens the guarded source of the prepared object, bracketed by binding checks.
    fn open(&self, cancel: &Cancellation) -> Result<Source, FsError> {
        self.revalidate_prepared(cancel)?;
        let mut source = Source::open_with_access(
            Parent {
                file: self.parent.clone(),
                path: self.parent_path.clone(),
                identity: Some(self.parent_identity),
                ancestry: None,
            },
            &self.name,
            None,
            Access::Copy(self.links),
            cancel,
        )?;
        // The guarded handles report to the job that prepared the item (the same objects),
        // even when the first use runs outside that job's namespace context.
        source._namespace_owner = self._namespace_owner.clone();
        // An item replaced by a link to the same folder is another binding.
        if source.native_original_identity() != Some(self.entry.identity) {
            return Err(invalid(self.path(), "source identity changed"));
        }
        self.check(source.entry.identity, source.entry.item.kind, source.stamp)?;
        source.revalidate(cancel)?;
        Ok(source)
    }

    fn with_active<T>(
        &self,
        cancel: &Cancellation,
        operation: impl FnOnce(&mut Source) -> Result<T, FsError>,
    ) -> Result<T, FsError> {
        let mut active = self.active.lock().map_err(|_| self.unavailable())?;
        let source = match &mut *active {
            Some(source) => source,
            empty => empty.insert(self.open(cancel)?),
        };
        operation(source)
    }
}

impl TransferSource for Prepared {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        super::super::namespace::source_pin(
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
        self.with_active(cancel, |source| source.discover(cancel, visit))
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.with_active(cancel, |source| source.open_child(name, cancel))
    }
    fn next_move_child(
        &mut self,
        _cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMove>, FsError> {
        Err(unsupported(
            self.path(),
            "Copy sources have no Move authority",
        ))
    }
    fn next_move_child_with_links(
        &mut self,
        _links: CopyLinkPolicy,
        _cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMoveEntry>, FsError> {
        Err(unsupported(
            self.path(),
            "Copy sources have no Move authority",
        ))
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferModifiedTimePin>, FsError> {
        self.with_active(cancel, |source| source.prepare_modified_time(cancel))
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        self.with_active(cancel, |source| source.rewind_for_validation(cancel))
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.with_active(cancel, |source| source.seek_for_validation(offset, cancel))
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let active = self.active.lock().map_err(|_| self.unavailable())?;
        match active.as_ref() {
            Some(source) => source.revalidate(cancel),
            None => self.revalidate_prepared(cancel),
        }
    }
    fn open_for_reading(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.with_active(cancel, |_| Ok(()))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.with_active(cancel, |source| source.read_chunk(buffer, cancel))
    }
    fn supports_read_retry(&self) -> bool {
        self.entry.item.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.with_active(cancel, |source| source.retry_read_from(offset, cancel))
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.with_active(cancel, |source| source.next_child(cancel))
    }
    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::TransferChild>, FsError> {
        self.with_active(cancel, |source| source.next_child_or_failure(cancel))
    }
    fn next_listed_child(
        &mut self,
        open: &dyn Fn(EntryKind) -> bool,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::ListedChild>, FsError> {
        self.with_active(cancel, |source| source.next_listed_child(open, cancel))
    }
}
