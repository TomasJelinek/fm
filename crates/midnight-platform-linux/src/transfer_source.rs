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
    /// How `revalidate` compares `stamp`; children and Move children inherit it.
    revisions: TransferRevisionPolicy,
    entries: Option<Dir>,
    move_enabled: bool,
    namespace: Arc<namespace::Bindings>,
    /// The selected root is the root of another mount than its parent.
    mount_root: bool,
    /// Bindings shared by this directory's children.
    children: namespace::ChildBindings,
    traversal: Option<directory_links::Traversal>,
    projected_link: Option<Arc<directory_links::Binding>>,
    /// A link transferred as a link (Preserve, or a link FollowLinks cannot follow).
    leaf: Option<directory_links::Leaf>,
    /// A folder the transfer entered holds descriptors: one of the process's levels.
    _level: Option<descriptor_budget::Level>,
    /// This folder as the folder that its Move children leave, checked once before the
    /// first of them (`moved_folder`).
    moved_folder: Option<super::move_source::SourceFolder>,
}
impl Source {
    pub(super) fn enable_move(&mut self) {
        self.move_enabled = true;
    }
    /// Copy and Move sources accept a moved ctime alone (VIEW-05); sources opened by name
    /// (checksums, file lists, viewer resources), drag previews and Rename keep the exact
    /// default.
    pub(super) fn with_revisions(mut self, revisions: TransferRevisionPolicy) -> Self {
        self.revisions = revisions;
        self
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    pub(super) fn original_link(&self) -> Option<Arc<directory_links::OriginalLink>> {
        self.projected_link
            .as_ref()
            .map(|binding| binding.original_link())
    }
    /// The link itself, when this source transfers a link as a link.
    pub(super) fn link_leaf(&self) -> Option<Arc<directory_links::OriginalLink>> {
        self.leaf.as_ref().map(|leaf| leaf.original.clone())
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
        self.child_source_in(self.child_namespace()?, name, cancel)
    }
    /// `namespace` is this directory's `child_namespace`.
    fn child_source_in(
        &self,
        namespace: Arc<namespace::Bindings>,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let mut source = Self::open_below(
            self.file.clone(),
            self.entry.item.path.clone(),
            name,
            Some(namespace),
            self.traversal.clone(),
            cancel,
        )?
        .with_revisions(self.revisions);
        // A folder the transfer enters holds its descriptors until it is done there.
        if source.entry.item.kind == EntryKind::Directory && source.leaf.is_none() {
            source._level = Some(descriptor_budget::levels().enter(&source.entry.item.path)?);
        }
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
            leaf,
        } = directory_links::probe(
            &parent,
            &parent_path,
            name,
            if mount_root { ROOT_RESOLVE } else { RESOLVE },
            traversal.as_mut(),
            cancel,
        )?;
        let path = &snapshot.item.path;
        if leaf.is_none() && !matches!(snapshot.item.kind, EntryKind::File | EntryKind::Directory) {
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
            revisions: TransferRevisionPolicy::Exact,
            entries: None,
            move_enabled: false,
            namespace,
            mount_root,
            children: namespace::ChildBindings::default(),
            traversal,
            projected_link,
            leaf,
            _level: None,
            moved_folder: None,
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
        // A link leaf has no content: its target is never opened.
        if !self.readable && !self.empty_link() && self.leaf.is_none() {
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
            // As deep as the transfer itself goes: each folder level counts against the
            // transfers' descriptor budget while discovery is inside it.
            crate::mutation::discovery::walk_with(
                &self.file,
                &self.entry.item.path,
                cancel,
                visit,
                &mut |path, _| {
                    descriptor_budget::levels()
                        .enter(path)
                        .map(|level| Some(Box::new(level) as Box<dyn std::any::Any>))
                },
            )?;
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
        let Some(name) = self.next_name(cancel)? else {
            return Ok(None);
        };
        let revisions = self.revisions;
        self.moved_folder(cancel)?
            .prepare_child(&name, revisions, cancel)
            .map(Some)
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
        let Some(name) = self.next_name(cancel)? else {
            return Ok(None);
        };
        let revisions = self.revisions;
        self.moved_folder(cancel)?
            .prepare_entry(&name, Some(traversal), revisions, cancel)
            .map(Some)
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::TransferMoveChild>, FsError> {
        let traversal = self.move_traversal(links)?;
        let Some(name) = self.next_name(cancel)? else {
            return Ok(None);
        };
        // The folder's own checks failing ends its enumeration; only preparing the
        // child is the child's failure.
        let revisions = self.revisions;
        let prepared = self
            .moved_folder(cancel)?
            .prepare_entry(&name, traversal, revisions, cancel);
        Ok(Some(prepared.map_err(|error| {
            midnight_core::TransferChildFailure { name, error }
        })))
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMoveEntry, FsError> {
        let traversal = self.move_traversal(links)?;
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(
                &self.entry.item.path,
                "named child requires a directory",
            ));
        }
        // Children are prepared below the folder's readable descriptor, which its
        // enumeration opened before any child could fail.
        let file = if self.readable {
            self.file.clone()
        } else {
            Arc::new(open_retained(&self.file, &self.entry.item.path)?)
        };
        super::move_source::prepare_child_or_failure(
            file,
            self.entry.item.path.clone(),
            name,
            Some(self.child_namespace()?),
            traversal,
            self.revisions,
            cancel,
        )?
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
            // The binding chain checked the link and its target; a followed file's
            // content must also be the revision that was captured.
            if self.entry.item.kind == EntryKind::File
                && revision_of(&self.file.metadata().map_err(|e| error(path, e))?) != self.stamp
            {
                return Err(invalid(path, "source content changed"));
            }
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
        if self.entry.item.kind == EntryKind::File
            && !revision_matches(self.stamp, revision_of(&metadata), self.revisions)
        {
            return Err(invalid(path, "source content changed"));
        }
        if self.leaf.is_some() && revision_of(&metadata) != self.stamp {
            return Err(invalid(path, "the link changed"));
        }
        Ok(())
    }
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferLinkRepresentation, FsError> {
        let path = &self.entry.item.path;
        let leaf = self
            .leaf
            .as_ref()
            .ok_or_else(|| unsupported(path, "this source is not a link kept as a link"))?;
        self.revalidate(cancel)?;
        // An independent no-follow read of the visible name: the same link object,
        // revision and exact text as captured. The target is never opened.
        leaf.original.verify_original(cancel)?;
        midnight_core::TransferLinkRepresentation::linux_symbolic_link(leaf.original.payload())
            .map_err(|mut failure| {
                failure.path = path.clone();
                failure
            })
    }
    fn link_not_followed(&self) -> Option<String> {
        self.leaf.as_ref().and_then(|leaf| leaf.unfollowed.clone())
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
    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::TransferChild>, FsError> {
        if self.entries.is_none() {
            self.check_children_open()?;
        }
        let Some(name) = self.next_name(cancel)? else {
            return Ok(None);
        };
        // The folder's own bindings failing ends its enumeration; only opening the
        // child is the child's failure.
        let namespace = self.child_namespace()?;
        Ok(Some(
            self.child_source_in(namespace, &name, cancel)
                .map(|source| Box::new(source) as Box<dyn TransferSource>)
                .map_err(|error| midnight_core::TransferChildFailure { name, error }),
        ))
    }
    /// A child that is not opened is described by its no-follow status, read relative to
    /// this folder's descriptor: no file is opened to be listed. A folder on another
    /// device (a mount point, which `RESOLVE` would refuse to enter) is described too.
    fn next_listed_child(
        &mut self,
        open: &dyn Fn(EntryKind) -> bool,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::ListedChild>, FsError> {
        use midnight_core::ListedChild;
        use rustix::fs::FileType;
        if self.traversal.is_some() {
            // A directory-link traversal decides a child by following it.
            return self
                .next_child(cancel)
                .map(|child| child.map(ListedChild::Source));
        }
        let Some((name, typed)) = self.next_typed_name(cancel)? else {
            return Ok(None);
        };
        // Opened as `next_child` opens it, with no status read first, when the type its
        // directory entry records already decides it (a folder's device is still read).
        let typed = match typed {
            FileType::RegularFile => Some(EntryKind::File),
            FileType::Symlink => Some(EntryKind::Symlink),
            FileType::Directory | FileType::Unknown => None,
            _ => Some(EntryKind::Other),
        };
        if typed.is_some_and(open) {
            return self
                .child_source(&name, cancel)
                .map(|source| Some(ListedChild::Source(Box::new(source))));
        }
        let path = self.entry.item.path.join(&name);
        let listed = super::listed_entry(&self.file, &name, &path)?;
        let mount = listed.item.kind == EntryKind::Directory
            && listed.identity.device != self.entry.identity.device;
        if open(listed.item.kind) && !mount {
            return self
                .child_source(&name, cancel)
                .map(|source| Some(ListedChild::Source(Box::new(source))));
        }
        Ok(Some(ListedChild::Entry(listed)))
    }
}
impl Source {
    /// A folder that lists its names but does not let them be opened (read without
    /// search permission) would fail for every child alike: that is the folder's
    /// own failure, reported once.
    fn check_children_open(&self) -> Result<(), FsError> {
        if self.entry.item.kind != EntryKind::Directory || self.empty_link() {
            return Ok(());
        }
        rustix::fs::accessat(
            &*self.file,
            ".",
            rustix::fs::Access::EXEC_OK,
            rustix::fs::AtFlags::EACCESS,
        )
        .map_err(|e| error(&self.entry.item.path, e.into()))
    }
    /// This folder as the folder that its Move children leave: checked once, before the
    /// first of them, for what every child would fail alike (`check_moved_folder`),
    /// which is then the folder's own failure. A failed check is not kept, so the next
    /// call checks again.
    fn moved_folder(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<&super::move_source::SourceFolder, FsError> {
        let folder = match self.moved_folder.take() {
            Some(folder) => folder,
            None => super::move_source::SourceFolder::moved(
                self.file.clone(),
                self.entry.item.path.clone(),
                self.child_namespace()?,
                cancel,
            )?,
        };
        Ok(self.moved_folder.insert(folder))
    }
    /// The link traversal that a Move child of this folder is prepared under for
    /// `links` (none for `Reject`); only a source prepared for Move has one.
    fn move_traversal(
        &self,
        links: CopyLinkPolicy,
    ) -> Result<Option<directory_links::Traversal>, FsError> {
        if !self.move_enabled {
            return Err(unsupported(
                &self.entry.item.path,
                "Copy sources have no Move authority",
            ));
        }
        if links == CopyLinkPolicy::Reject {
            return Ok(None);
        }
        self.traversal
            .as_ref()
            .filter(|traversal| traversal.matches_policy(links))
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                unsupported(
                    &self.entry.item.path,
                    "this source has no Move authority for the requested directory link policy",
                )
            })
    }
    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<OsString>, FsError> {
        Ok(self.next_typed_name(cancel)?.map(|(name, _)| name))
    }
    /// The next name and the type its directory entry gives (`Unknown` where the
    /// filesystem does not record types in its directories).
    fn next_typed_name(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<(OsString, rustix::fs::FileType)>, FsError> {
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
            return Ok(Some((name.to_owned(), next.file_type())));
        }
    }
}

#[cfg(test)]
#[path = "transfer_source_child_tests.rs"]
mod child_tests;
