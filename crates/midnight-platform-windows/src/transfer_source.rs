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

#[path = "transfer_source_root.rs"]
pub(super) mod root;

pub(super) struct Source {
    entry: TransferEntry,
    native: Native,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    readable: Option<File>,
    // Size, LastWriteTime and creation time; ChangeTime is not part of it.
    stamp: (u64, u64, u64),
    listing: Listing,
    access: Access,
    /// How the Move capture of each Move child compares the child's pinned revision.
    move_revisions: TransferRevisionPolicy,
    link: Option<Arc<source_links::Binding>>,
    parent_ancestry: Option<Arc<Ancestor>>,
    directory_ancestry: Option<Arc<Ancestor>>,
    // Last: all native fields, including lazy readable upgrades, close before this signal.
    _namespace_owner: super::namespace::NativeOwner,
}
/// A source's directory and its already proven identity, for opening its children.
#[derive(Clone)]
struct Parent {
    file: Arc<File>,
    path: PathBuf,
    identity: Option<TransferIdentity>,
    ancestry: Option<Arc<Ancestor>>,
}

struct Ancestor {
    _directory: Arc<File>,
    identity: TransferIdentity,
    parent: Parent,
    link: Option<Arc<source_links::Binding>>,
    contains_link: bool,
}

#[derive(Clone, Copy)]
enum Access {
    Copy(CopyLinkPolicy),
    Discover(CopyLinkPolicy),
    Move(CopyLinkPolicy),
}

impl Access {
    fn links(self) -> CopyLinkPolicy {
        match self {
            Self::Copy(links) | Self::Discover(links) | Self::Move(links) => links,
        }
    }

    fn projects_links(self) -> bool {
        matches!(
            self.links(),
            CopyLinkPolicy::FollowDirectoryContents | CopyLinkPolicy::EmptyDirectories
        )
    }
}

#[derive(Default)]
struct Listing {
    names: VecDeque<(OsString, Native)>,
    cursor: Cursor,
}

impl Listing {
    fn fill(&mut self, directory: &File, path: &Path) -> Result<(), FsError> {
        let names = &mut self.names;
        self.cursor
            .next_batch(directory, &mut |record| {
                names.push_back((OsString::from_wide(record.name), record.native));
                Ok(())
            })
            .map_err(|e| error(path, e))
    }
}

impl Source {
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_links(parent, parent_path, name, CopyLinkPolicy::Reject, cancel)
    }
    pub(super) fn open_with_links(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            Parent {
                file: parent,
                path: parent_path,
                identity: None,
                ancestry: None,
            },
            name,
            None,
            Access::Copy(links),
            cancel,
        )
    }
    pub(super) fn open_for_move(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            Parent {
                file: parent,
                path: parent_path,
                identity: None,
                ancestry: None,
            },
            name,
            None,
            Access::Move(links),
            cancel,
        )
    }
    /// The revision policy of this Move source's children's Move captures.
    pub(super) fn with_move_revisions(mut self, revisions: TransferRevisionPolicy) -> Self {
        self.move_revisions = revisions;
        self
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    pub(super) fn original_link(&self) -> Option<Arc<source_links::OriginalLink>> {
        self.link.as_ref().map(|binding| binding.original_link())
    }
    /// `listed` is the child's classification from its directory record. Without it, a
    /// no-follow probe classifies the entry first.
    fn open_with_access(
        mut parent: Parent,
        name: &OsStr,
        listed: Option<Native>,
        access: Access,
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
        let expected_kind = expected.kind();
        let (file, link) = if expected_kind == EntryKind::DirectoryLink
            && !matches!(access, Access::Move(_))
            && access.projects_links()
        {
            let (file, link) = source_links::Binding::capture(
                parent.file.clone(),
                name,
                &path,
                expected,
                probed,
                access.links() == CopyLinkPolicy::FollowDirectoryContents,
                cancel,
            )?;
            (file, Some(link))
        } else {
            let file = open_ordinary(&parent.file, name, &path, expected_kind, access)?;
            (Arc::new(file), None)
        };
        let (mut entry, native, meta) = classified_with_metadata(&file, &path)?;
        if let Some(link) = &link {
            if link.is_empty() {
                if native.kind() != EntryKind::DirectoryLink {
                    return Err(invalid(&path, "directory link changed while opening"));
                }
                entry.item.kind = EntryKind::Directory;
                entry.item.metadata.native_attributes =
                    Some(native.attributes & !FILE_ATTRIBUTE_REPARSE_POINT);
            } else if native.kind() != EntryKind::Directory {
                return Err(invalid(
                    &path,
                    "directory link target changed while opening",
                ));
            }
        } else if probed.is_some_and(|id| id != entry.identity) || native.kind() != expected_kind {
            return Err(invalid(&path, "source changed while opening"));
        }
        let stamp = (
            meta.file_size(),
            meta.last_write_time(),
            meta.creation_time(),
        );
        // A cloud placeholder or deduplicated/WOF file is read through its filter so its
        // data hydrates instead of copying (and then verifying) the raw reparse stub.
        let readable =
            if matches!(access, Access::Copy(_)) && native.filtered() && !native.directory() {
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
        parent.identity = Some(parent_identity);
        if link.as_ref().is_some_and(|link| !link.is_empty())
            && (entry.identity == parent_identity
                || contains_identity(parent.ancestry.as_deref(), entry.identity))
        {
            return Err(invalid(
                &path,
                "directory link forms a cycle in the Copy source",
            ));
        }
        let directory_ancestry = (entry.item.kind == EntryKind::Directory
            && access.links() == CopyLinkPolicy::FollowDirectoryContents)
            .then(|| {
                Arc::new(Ancestor {
                    _directory: file.clone(),
                    identity: entry.identity,
                    parent: parent.clone(),
                    link: link.clone(),
                    contains_link: link.is_some() || has_followed_link(parent.ancestry.as_deref()),
                })
            });
        let namespace_owner = super::namespace::owner(&[parent_identity, entry.identity]);
        Ok(Self {
            entry,
            native,
            parent: parent.file,
            parent_path: parent.path,
            parent_identity,
            name: name.to_owned(),
            file,
            readable,
            stamp,
            listing: Listing::default(),
            access,
            move_revisions: TransferRevisionPolicy::Exact,
            link,
            parent_ancestry: parent.ancestry,
            directory_ancestry,
            _namespace_owner: namespace_owner,
        })
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        self.ensure_readable()?;
        let directory = self.readable.as_ref().unwrap_or(&self.file);
        self.listing.fill(directory, &self.entry.item.path)
    }
    /// Opens a listed child relative to this directory, reusing its proven identity.
    fn open_listed(
        &self,
        name: &OsStr,
        native: Native,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(
            self.child_parent(),
            name,
            Some(native),
            Access::Copy(self.access.links()),
            cancel,
        )
    }

    fn child_parent(&self) -> Parent {
        Parent {
            file: self.file.clone(),
            path: self.entry.item.path.clone(),
            identity: Some(self.entry.identity),
            ancestry: self.directory_ancestry.clone(),
        }
    }

    fn empty_projection(&self) -> bool {
        self.link.as_ref().is_some_and(|link| link.is_empty())
    }
}
impl TransferSource for Source {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        if self.link.is_some() || has_followed_link(self.parent_ancestry.as_deref()) {
            // A logical name alone cannot recapture the original object and its target.
            return Ok(None);
        }
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
        if self.access.projects_links() {
            self.discover_projection(cancel, visit)?;
            return self.revalidate(cancel);
        }
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
        if self.empty_projection() {
            return Err(error(
                &self.entry.item.path.join(name),
                io::ErrorKind::NotFound.into(),
            ));
        }
        Self::open_with_access(
            self.child_parent(),
            name,
            None,
            Access::Copy(self.access.links()),
            cancel,
        )
        .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMove>, FsError> {
        if !matches!(self.access, Access::Move(_)) {
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
                    self.move_revisions,
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
        use midnight_core::PreparedTransferMoveEntry;
        if links == CopyLinkPolicy::Reject {
            return self
                .next_move_child(cancel)
                .map(|child| child.map(PreparedTransferMoveEntry::Ordinary));
        }
        if !matches!(self.access, Access::Move(_)) || !Access::Copy(links).projects_links() {
            return Err(unsupported(
                &self.entry.item.path,
                "recursive Move with directory links is unsupported",
            ));
        }
        self.next_name(cancel)?
            .map(|(name, listed)| {
                super::move_source::prepare_child_with_links(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    Some(listed),
                    links,
                    self.move_revisions,
                    cancel,
                )
            })
            .transpose()
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::TransferMoveChild>, FsError> {
        self.check_move_children(links)?;
        let Some((name, listed)) = self.next_name(cancel)? else {
            return Ok(None);
        };
        // A Windows child is prepared below the folder's held handle with nothing that
        // every child would fail alike, so each preparation failure is the child's.
        Ok(Some(
            self.prepare_move_entry(&name, Some(listed), links, cancel)
                .map_err(|error| midnight_core::TransferChildFailure { name, error }),
        ))
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMoveEntry, FsError> {
        self.check_move_children(links)?;
        self.revalidate(cancel)?;
        self.prepare_move_entry(name, None, links, cancel)
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
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        self.link
            .as_ref()
            .map(|link| link.original_identity())
            .or_else(|| self.native_identity())
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        revalidate_parent(
            &self.parent_path,
            self.parent_identity,
            self.parent_ancestry.as_deref(),
            path,
            cancel,
        )?;
        if let Some(link) = &self.link {
            return link.revalidate(cancel);
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
    /// Reading this folder (its binding, its listing) and a cancellation fail the folder
    /// and end its enumeration; only opening a listed child is that child's failure.
    /// Unlike Linux, there is no folder whose names can be listed but not opened: a
    /// folder is held with list and traverse access, and each child's own access
    /// (an access-denied file, a file another program writes) decides its open.
    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::TransferChild>, FsError> {
        checked(cancel, &self.entry.item.path)?;
        let Some((name, native)) = self.next_name(cancel)? else {
            return Ok(None);
        };
        Ok(Some(
            self.open_listed(&name, native, cancel)
                .map(|source| Box::new(source) as Box<dyn TransferSource>)
                .map_err(|error| midnight_core::TransferChildFailure { name, error }),
        ))
    }
}
impl Source {
    fn read_file(&self) -> &File {
        self.readable.as_ref().unwrap_or(&self.file)
    }

    fn ensure_readable(&mut self) -> Result<(), FsError> {
        if !matches!(self.access, Access::Move(_)) || self.readable.is_some() {
            return Ok(());
        }
        if self.entry.item.kind == EntryKind::Directory {
            // Shares the original handle's DELETE access and, as every folder handle of a
            // transfer does, writing (see `held_sharing`).
            let held = native::open_relative(
                &self.parent,
                &self.name,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
                held_sharing(EntryKind::Directory) | FILE_SHARE_DELETE,
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

    /// The refusals of `next_move_child` and `next_move_child_with_links` for `links`.
    fn check_move_children(&self, links: CopyLinkPolicy) -> Result<(), FsError> {
        if links == CopyLinkPolicy::Reject {
            if !matches!(self.access, Access::Move(_)) {
                return Err(unsupported(
                    &self.entry.item.path,
                    "Copy sources have no Move authority",
                ));
            }
        } else if !matches!(self.access, Access::Move(_)) || !Access::Copy(links).projects_links() {
            return Err(unsupported(
                &self.entry.item.path,
                "recursive Move with directory links is unsupported",
            ));
        }
        Ok(())
    }

    /// The Move child `name` (`listed` as its folder's listing read it), under `links`.
    fn prepare_move_entry(
        &self,
        name: &OsStr,
        listed: Option<Native>,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMoveEntry, FsError> {
        if links == CopyLinkPolicy::Reject {
            return super::move_source::prepare_child(
                self.file.clone(),
                self.entry.item.path.clone(),
                name,
                self.move_revisions,
                cancel,
            )
            .map(midnight_core::PreparedTransferMoveEntry::Ordinary);
        }
        super::move_source::prepare_child_with_links(
            self.file.clone(),
            self.entry.item.path.clone(),
            name,
            listed,
            links,
            self.move_revisions,
            cancel,
        )
    }

    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<(OsString, Native)>, FsError> {
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        if self.empty_projection() {
            self.revalidate(cancel)?;
            return Ok(None);
        }
        loop {
            if let Some(next) = self.listing.names.pop_front() {
                return Ok(Some(next));
            }
            // The directory's path binding is revalidated once per native batch and once
            // at its end. Each child is opened relative to the retained handle and the
            // engine revalidates every child's own parent binding before using it.
            self.revalidate(cancel)?;
            if self.listing.cursor.ended() {
                return Ok(None);
            }
            self.fill_names()?;
        }
    }
}

fn open_ordinary(
    parent: &File,
    name: &OsStr,
    path: &Path,
    kind: EntryKind,
    access: Access,
) -> Result<File, FsError> {
    let (options, directory_access) = match kind {
        EntryKind::Directory => (FILE_DIRECTORY_FILE, FILE_LIST_DIRECTORY | FILE_TRAVERSE),
        EntryKind::File => (FILE_NON_DIRECTORY_FILE, 0),
        _ => {
            return Err(unsupported(
                path,
                "copying links and special entries is not supported",
            ));
        }
    };
    let access = FILE_READ_ATTRIBUTES
        | SYNCHRONIZE
        | match access {
            Access::Move(_) => DELETE | (directory_access & FILE_TRAVERSE),
            Access::Copy(_) => FILE_READ_DATA | directory_access,
            Access::Discover(_) => directory_access,
        };
    native::open_relative(parent, name, access, held_sharing(kind), FILE_OPEN, options)
        .map_err(|e| error(path, e))
}

/// The sharing of a source handle that guards an entry while it is transferred. A file
/// denies write and delete sharing: its content and name cannot change while it is read
/// and verified. A folder denies only delete sharing, which keeps its name. Windows
/// refuses every rename into a folder (another job publishing a copied file, a program
/// saving by rename) while any handle on it denies write sharing, and that would not
/// freeze its listing anyway: creating and deleting entries is never refused by sharing.
/// Each child is checked against the listing and guarded by its own handle instead.
fn held_sharing(kind: EntryKind) -> u32 {
    if kind == EntryKind::Directory {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    } else {
        FILE_SHARE_READ
    }
}

fn contains_identity(mut ancestor: Option<&Ancestor>, id: TransferIdentity) -> bool {
    while let Some(directory) = ancestor {
        if directory.identity == id {
            return true;
        }
        ancestor = directory.parent.ancestry.as_deref();
    }
    false
}

fn has_followed_link(ancestor: Option<&Ancestor>) -> bool {
    ancestor.is_some_and(|ancestor| ancestor.contains_link)
}

fn revalidate_parent<'a>(
    mut parent_path: &'a Path,
    mut parent_identity: TransferIdentity,
    mut ancestor: Option<&'a Ancestor>,
    path: &Path,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    if has_followed_link(ancestor) {
        // These ordinary directory handles remain pinned with deny-delete sharing.
        // Revalidate redirecting links against their own held parents instead of resolving
        // a raw path through the redirects again.
        while let Some(directory) = ancestor {
            if let Some(link) = &directory.link {
                link.revalidate(cancel)?;
            }
            parent_path = &directory.parent.path;
            parent_identity = directory.parent.identity.ok_or_else(|| {
                invalid(
                    path,
                    "Copy source ancestry lacks a retained parent identity",
                )
            })?;
            ancestor = directory.parent.ancestry.as_deref();
        }
    }
    let visible = native::open_parent(parent_path).map_err(|e| error(path, e))?;
    if identity(&visible, path)? != parent_identity {
        return Err(invalid(path, "source parent changed"));
    }
    Ok(())
}

struct DiscoveryFrame {
    parent: Parent,
    listing: Listing,
}

impl DiscoveryFrame {
    fn open(source: &Source, cancel: &Cancellation) -> Result<Self, FsError> {
        source.revalidate(cancel)?;
        let mut parent = source.child_parent();
        // Start a new kernel enumeration. A duplicate of the Copy handle would share
        // its enumeration position and could skip entries when the actual Copy starts.
        // Discovery runs while other jobs may publish into the folders it lists.
        let share = held_sharing(EntryKind::Directory)
            | if matches!(source.access, Access::Move(_)) {
                FILE_SHARE_DELETE
            } else {
                0
            };
        let file = native::reopen(
            &parent.file,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE,
            share,
        )
        .map_err(|error| super::error(&parent.path, error))?;
        parent.file = Arc::new(file);
        Ok(Self {
            parent,
            listing: Listing::default(),
        })
    }
}

impl Source {
    fn discover_projection(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        visit(&self.entry.item);
        if self.entry.item.kind != EntryKind::Directory || self.empty_projection() {
            return Ok(());
        }
        let mut stack = vec![DiscoveryFrame::open(self, cancel)?];
        while let Some(frame) = stack.last_mut() {
            checked(cancel, &frame.parent.path)?;
            if let Some((name, native)) = frame.listing.names.pop_front() {
                let child = Self::open_with_access(
                    frame.parent.clone(),
                    &name,
                    Some(native),
                    Access::Discover(self.access.links()),
                    cancel,
                )?;
                visit(&child.entry.item);
                if child.entry.item.kind == EntryKind::Directory && !child.empty_projection() {
                    if stack.len() >= 128 {
                        return Err(unsupported(
                            &child.entry.item.path,
                            "Discovery exceeds the 128-directory depth budget",
                        ));
                    }
                    stack.push(DiscoveryFrame::open(&child, cancel)?);
                }
            } else if frame.listing.cursor.ended() {
                stack.pop();
            } else {
                let path = &frame.parent.path;
                let id = frame.parent.identity.ok_or_else(|| {
                    invalid(path, "Copy discovery lacks a retained directory identity")
                })?;
                revalidate_parent(path, id, frame.parent.ancestry.as_deref(), path, cancel)?;
                frame.listing.fill(&frame.parent.file, path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "transfer_source_child_tests.rs"]
mod child_tests;
#[cfg(test)]
#[path = "transfer_source_sharing_tests.rs"]
mod sharing_tests;
#[cfg(test)]
#[path = "transfer_source_volume_tests.rs"]
mod volume_tests;
