use super::*;
use rustix::fs::Dir;
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    os::fd::AsRawFd,
};

/// A selected Copy root may itself be a mount point (a volume selected in its
/// parent's listing); its descendants never cross into another mount.
const ROOT_RESOLVE: ResolveFlags = ResolveFlags::BENEATH.union(ResolveFlags::NO_SYMLINKS);

pub(super) struct Source {
    entry: TransferEntry,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    readable: bool,
    stamp: (u64, i64, i64, i64, i64),
    entries: Option<Dir>,
    move_enabled: bool,
    namespace: Arc<namespace::Bindings>,
    /// The selected root is the root of another mount than its parent.
    mount_root: bool,
    /// Bindings shared by this directory's children.
    children: namespace::ChildBindings,
    traversal: Option<directory_links::Traversal>,
    projected_link: Option<Arc<directory_links::Binding>>,
}
impl Source {
    pub(super) fn enable_move(&mut self) {
        self.move_enabled = true;
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    pub(super) fn original_link(&self) -> Option<Arc<directory_links::OriginalLink>> {
        self.projected_link
            .as_ref()
            .map(|binding| binding.original_link())
    }
    pub(super) fn namespace_bindings(&self) -> Arc<namespace::Bindings> {
        self.namespace.clone()
    }
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let mut source = Self::open_move(parent, parent_path, name, cancel)?;
        source.make_readable(cancel)?;
        Ok(source)
    }
    /// A selected Copy root, which may be the root of a mounted filesystem.
    pub(super) fn open_root(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        traversal: Option<directory_links::Traversal>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let attempt = Self::open_with(
            parent.clone(),
            parent_path.clone(),
            name,
            false,
            None,
            traversal.clone(),
            cancel,
        );
        let mut source = match attempt {
            Err(failure) if failure.kind == io::ErrorKind::CrossesDevices => {
                Self::open_with(parent, parent_path, name, true, None, traversal, cancel)?
            }
            result => result?,
        };
        source.make_readable(cancel)?;
        Ok(source)
    }
    /// Retain only metadata authority; native Move never opens file contents.
    pub(super) fn open_move(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_move_with(parent, parent_path, name, None, cancel)
    }
    /// `namespace` reuses bindings already computed for the parent directory.
    pub(super) fn open_move_with(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        namespace: Option<Arc<namespace::Bindings>>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_below(parent, parent_path, name, namespace, None, cancel)
    }
    pub(super) fn open_below(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        namespace: Option<Arc<namespace::Bindings>>,
        traversal: Option<directory_links::Traversal>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with(
            parent,
            parent_path,
            name,
            false,
            namespace,
            traversal,
            cancel,
        )
        .map_err(|mut failure| {
            if failure.kind == io::ErrorKind::CrossesDevices {
                failure.kind = io::ErrorKind::Unsupported;
                failure.message = "This entry is a mount point. Move never crosses mounts, and \
                                   Copy does not enter another mount below a selected folder."
                    .into();
            }
            failure
        })
    }
    /// The bindings of this directory, shared by all of its children.
    pub(super) fn child_namespace(&self) -> Result<Arc<namespace::Bindings>, FsError> {
        self.children.get(&self.file, &self.entry.item.path)
    }
    fn child_source(&self, name: &OsStr, cancel: &Cancellation) -> Result<Self, FsError> {
        let namespace = self.child_namespace()?;
        let mut source = Self::open_below(
            self.file.clone(),
            self.entry.item.path.clone(),
            name,
            Some(namespace),
            self.traversal.clone(),
            cancel,
        )?;
        source.make_readable(cancel)?;
        Ok(source)
    }
    fn open_with(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        mount_root: bool,
        namespace: Option<Arc<namespace::Bindings>>,
        mut traversal: Option<directory_links::Traversal>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let directory_links::Probe {
            entry: snapshot,
            file,
            projected_link,
        } = directory_links::probe(
            &parent,
            &parent_path,
            name,
            if mount_root { ROOT_RESOLVE } else { RESOLVE },
            traversal.as_mut(),
            cancel,
        )?;
        let path = &snapshot.item.path;
        if !matches!(snapshot.item.kind, EntryKind::File | EntryKind::Directory) {
            return Err(unsupported(
                path,
                "copying links and special entries is not supported",
            ));
        }
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        let stamp = revision_of(&metadata);
        let parent_identity = identity(&parent, &parent_path)?;
        let namespace = match namespace {
            Some(namespace) => namespace,
            None => namespace::Bindings::for_parent(&parent, &parent_path)?,
        };
        Ok(Self {
            entry: snapshot,
            parent,
            parent_path,
            parent_identity,
            name: name.to_owned(),
            file,
            readable: false,
            stamp,
            entries: None,
            move_enabled: false,
            namespace,
            mount_root,
            children: namespace::ChildBindings::default(),
            traversal,
            projected_link,
        })
    }
    fn empty_link(&self) -> bool {
        self.projected_link.is_some()
            && self
                .traversal
                .as_ref()
                .is_some_and(directory_links::Traversal::empty)
    }
    fn has_link_bindings(&self) -> bool {
        self.traversal
            .as_ref()
            .is_some_and(directory_links::Traversal::has_bindings)
    }
    fn make_readable(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        if !self.readable && !self.empty_link() {
            self.file = Arc::new(open_retained(&self.file, &self.entry.item.path)?);
            self.readable = true;
        }
        Ok(())
    }
}

/// Open the retained inode, never its former pathname, only when content access is needed.
pub(super) fn open_retained(file: &File, path: &Path) -> Result<File, FsError> {
    let expected = identity(file, path)?;
    let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
    let opened = File::from(
        openat(
            CWD,
            descriptor.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| error(path, e.into()))?,
    );
    if identity(&opened, path)? != expected {
        return Err(invalid(path, "retained source identity changed"));
    }
    Ok(opened)
}
impl TransferSource for Source {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        if self.mount_root || self.has_link_bindings() {
            // The ordinary endpoint pin cannot express these read scopes.
            return Ok(None);
        }
        let pin = self.namespace.pin(
            &self.parent,
            &self.parent_path,
            &self.name,
            self.entry.item.kind,
            true,
            cancel,
        )?;
        self.revalidate(cancel)?;
        Ok(Some(pin))
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if let Some(traversal) = &self.traversal {
            directory_links::discover(
                &self.file,
                &self.entry,
                traversal,
                self.empty_link(),
                cancel,
                visit,
            )?;
        } else {
            crate::mutation::discovery::walk(&self.file, &self.entry.item.path, cancel, visit)?;
        }
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
        if self.empty_link() {
            return Err(error(&self.entry.item.path, io::ErrorKind::NotFound.into()));
        }
        self.child_source(name, cancel)
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
            .map(|name| {
                super::move_source::prepare_child(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    self.child_namespace()?,
                    cancel,
                )
            })
            .transpose()
    }
    fn next_move_child_with_links(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMoveEntry>, FsError> {
        if links == CopyLinkPolicy::Reject {
            return self
                .next_move_child(cancel)
                .map(|child| child.map(midnight_core::PreparedTransferMoveEntry::Ordinary));
        }
        let traversal = self
            .traversal
            .as_ref()
            .filter(|traversal| self.move_enabled && traversal.matches_policy(links))
            .cloned()
            .ok_or_else(|| {
                unsupported(
                    &self.entry.item.path,
                    "this source has no Move authority for the requested directory link policy",
                )
            })?;
        self.next_name(cancel)?
            .map(|name| {
                super::move_source::prepare_child_with_links(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    Some(self.child_namespace()?),
                    traversal,
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
        super::metadata::prepare(self.file.clone(), &self.entry.item.path, cancel)
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
        self.make_readable(cancel)?;
        super::validation::rewind(&self.file, &self.entry.item.path, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.make_readable(cancel)?;
        let path = &self.entry.item.path;
        if self.entry.item.kind != EntryKind::File {
            return Err(invalid(path, "range validation requires a regular file"));
        }
        let length = self.file.metadata().map_err(|e| error(path, e))?.len();
        if offset > length {
            return Err(invalid(
                path,
                "validation offset exceeds retained file length",
            ));
        }
        let actual = (&*self.file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(path, e))?;
        if actual != offset {
            return Err(invalid(
                path,
                "retained file seek returned a different offset",
            ));
        }
        checked(cancel, path)?;
        Ok(length)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        Some(
            self.projected_link
                .as_ref()
                .map_or(self.entry.identity, |binding| binding.original_identity()),
        )
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        if let Some(traversal) = &self.traversal {
            traversal.revalidate(cancel)?;
        }
        if self.projected_link.is_some() {
            return Ok(());
        }
        let linked_parent = self
            .traversal
            .as_ref()
            .map(|traversal| traversal.visible_parent(&self.parent_path, path))
            .transpose()?
            .flatten();
        let visible_parent = match linked_parent {
            Some(parent) => parent,
            None => File::from(
                openat(CWD, &self.parent_path, DIRECTORY, Mode::empty())
                    .map_err(|e| error(path, e.into()))?,
            ),
        };
        if identity(&visible_parent, path)? != self.parent_identity {
            return Err(invalid(path, "source parent changed"));
        }
        let visible = File::from(
            openat2(
                &*self.parent,
                &self.name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                if self.mount_root {
                    ROOT_RESOLVE
                } else {
                    RESOLVE
                },
            )
            .map_err(|e| error(path, e.into()))?,
        );
        if identity(&visible, path)? != self.entry.identity {
            return Err(invalid(path, "source identity changed"));
        }
        let metadata = self.file.metadata().map_err(|e| error(path, e))?;
        if self.entry.item.kind == EntryKind::File && revision_of(&metadata) != self.stamp {
            return Err(invalid(path, "source content changed"));
        }
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.make_readable(cancel)?;
        if self.entry.item.kind != EntryKind::File {
            return Err(invalid(
                &self.entry.item.path,
                "directory content is not a byte stream",
            ));
        }
        (&*self.file)
            .read(buffer)
            .map_err(|e| error(&self.entry.item.path, e))
    }
    fn supports_read_retry(&self) -> bool {
        self.entry.item.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        self.make_readable(cancel)?;
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
        (&*self.file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(&self.entry.item.path, e))?;
        self.revalidate(cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.next_name(cancel)?
            .map(|name| {
                self.child_source(&name, cancel)
                    .map(|source| Box::new(source) as Box<dyn TransferSource>)
            })
            .transpose()
    }
}
impl Source {
    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<OsString>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        if self.empty_link() {
            return Ok(None);
        }
        if self.entries.is_none() {
            self.make_readable(cancel)?;
            self.entries = Some(
                Dir::read_from(&*self.file).map_err(|e| error(&self.entry.item.path, e.into()))?,
            );
        }
        loop {
            checked(cancel, &self.entry.item.path)?;
            let next = self.entries.as_mut().and_then(Iterator::next);
            let Some(next) = next else {
                return Ok(None);
            };
            let next = next.map_err(|e| error(&self.entry.item.path, e.into()))?;
            let name = OsStr::from_bytes(next.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            return Ok(Some(name.to_owned()));
        }
    }
}
