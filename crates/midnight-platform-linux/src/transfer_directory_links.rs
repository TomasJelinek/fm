//! Read-only Copy projections of qualified native directory links.

use super::destination::recovery::Revision;
use super::*;
use rustix::fs::{Dir, readlinkat};
use std::{ffi::OsString, io::Read, os::unix::ffi::OsStringExt};

const MAX_DEPTH: usize = 128;
const MAX_MOUNT_ANCESTORS: usize = 4096;
const MAX_MOUNT_METADATA: u64 = 4 * 1024 * 1024;
const PATH_DIRECTORY: OFlags = OFlags::PATH.union(OFlags::DIRECTORY).union(OFlags::CLOEXEC);
const LINK_RESOLVE: ResolveFlags = ResolveFlags::NO_MAGICLINKS.union(ResolveFlags::NO_XDEV);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectoryLinkMode {
    Follow,
    Empty,
}

#[derive(Clone)]
pub(super) struct Traversal {
    mode: DirectoryLinkMode,
    directories: Option<Arc<Ancestor>>,
    links: Option<Arc<Binding>>,
}

struct Ancestor {
    identity: TransferIdentity,
    depth: usize,
    previous: Option<Arc<Self>>,
}

impl Traversal {
    pub(super) fn new(mode: DirectoryLinkMode) -> Self {
        Self {
            mode,
            directories: None,
            links: None,
        }
    }

    pub(super) fn empty(&self) -> bool {
        self.mode == DirectoryLinkMode::Empty
    }

    pub(super) fn matches_policy(&self, links: CopyLinkPolicy) -> bool {
        matches!(
            (self.mode, links),
            (
                DirectoryLinkMode::Follow,
                CopyLinkPolicy::FollowDirectoryContents
            ) | (DirectoryLinkMode::Empty, CopyLinkPolicy::EmptyDirectories)
        )
    }

    pub(super) fn has_bindings(&self) -> bool {
        self.links.is_some()
    }

    fn enter_directory(
        &mut self,
        identity: TransferIdentity,
        path: &Path,
        projected_link: bool,
    ) -> Result<(), FsError> {
        let depth = self
            .directories
            .as_ref()
            .map_or(1, |parent| parent.depth + 1);
        if depth > MAX_DEPTH {
            return Err(unsupported(
                path,
                "Linked Copy exceeds the 128-directory traversal-depth budget",
            ));
        }
        if projected_link && self.mode == DirectoryLinkMode::Follow {
            let mut cursor = self.directories.as_deref();
            while let Some(ancestor) = cursor {
                if ancestor.identity == identity {
                    return Err(invalid(path, "directory link target is an active ancestor"));
                }
                cursor = ancestor.previous.as_deref();
            }
        }
        self.directories = Some(Arc::new(Ancestor {
            identity,
            depth,
            previous: self.directories.take(),
        }));
        Ok(())
    }

    pub(super) fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if let Some(binding) = &self.links {
            binding.revalidate_chain(cancel)?;
        }
        Ok(())
    }

    pub(super) fn visible_parent(
        &self,
        parent_path: &Path,
        logical_path: &Path,
    ) -> Result<Option<File>, FsError> {
        self.links
            .as_ref()
            .map(|binding| binding.original.mount.reopen(parent_path, logical_path))
            .transpose()
    }
}

pub(super) struct Probe {
    pub(super) entry: TransferEntry,
    pub(super) file: Arc<File>,
    pub(super) projected_link: Option<Arc<Binding>>,
}

pub(super) fn probe(
    parent: &Arc<File>,
    parent_path: &Path,
    name: &OsStr,
    resolve: ResolveFlags,
    traversal: Option<&mut Traversal>,
    cancel: &Cancellation,
) -> Result<Probe, FsError> {
    let path = parent_path.join(name);
    checked(cancel, &path)?;
    validate_name(name, &path)?;
    let mut file = Arc::new(File::from(
        openat2(
            &**parent,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            resolve,
        )
        .map_err(|failure| error(&path, failure.into()))?,
    ));
    let mut snapshot = entry(&file, &path)?;
    let mut projected_link = None;
    if let Some(traversal) = traversal {
        if snapshot.item.kind == EntryKind::Symlink {
            let binding = Arc::new(Binding::capture(
                parent,
                parent_path,
                name,
                file.clone(),
                snapshot,
                traversal.links.clone(),
                cancel,
            )?);
            snapshot = entry(&binding.target, &path)?;
            if traversal.empty() {
                snapshot.identity = binding.original_identity();
            } else {
                file = binding.target.clone();
            }
            traversal.links = Some(binding.clone());
            projected_link = Some(binding);
        }
        if snapshot.item.kind == EntryKind::Directory {
            traversal.enter_directory(snapshot.identity, &path, projected_link.is_some())?;
        }
    }
    Ok(Probe {
        entry: snapshot,
        file,
        projected_link,
    })
}

pub(super) struct Binding {
    original: Arc<OriginalLink>,
    target: Arc<File>,
    target_identity: TransferIdentity,
    previous: Option<Arc<Self>>,
}

/// Authority over the link itself. Binding owns the separate referent.
pub(super) struct OriginalLink {
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    mount: Arc<MountAnchor>,
    name: OsString,
    stub: Arc<File>,
    entry: TransferEntry,
    revision: Revision,
    payload: OsString,
}

impl Binding {
    fn capture(
        parent: &Arc<File>,
        parent_path: &Path,
        name: &OsStr,
        stub: Arc<File>,
        snapshot: TransferEntry,
        previous: Option<Arc<Self>>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        checked(cancel, &snapshot.item.path)?;
        let mount = match &previous {
            Some(binding) => binding.original.mount.clone(),
            None => Arc::new(MountAnchor::capture(parent, parent_path, cancel)?),
        };
        let original = Arc::new(OriginalLink::capture(
            parent,
            parent_path,
            name,
            stub,
            snapshot,
            mount,
        )?);
        let path = &original.entry.item.path;
        // Resolve the captured payload exactly and independently check the alias
        // belongs to the same target through NO_MAGICLINKS.
        let target = Arc::new(open_target(parent, &original.payload, path)?);
        let binding = Self {
            target_identity: identity(&target, path)?,
            original,
            target,
            previous,
        };
        binding.revalidate_here(cancel)?;
        Ok(binding)
    }

    pub(super) fn original_identity(&self) -> TransferIdentity {
        self.original.entry.identity
    }

    pub(super) fn original_link(&self) -> Arc<OriginalLink> {
        self.original.clone()
    }

    fn revalidate_chain(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if let Some(parent) = &self.previous {
            parent.revalidate_chain(cancel)?;
        }
        self.revalidate_here(cancel)
    }

    fn revalidate_here(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.original.revalidate(cancel)?;
        let path = &self.original.entry.item.path;
        let target = open_target(&self.original.parent, &self.original.name, path)?;
        if identity(&target, path)? != self.target_identity {
            return Err(invalid(path, "directory link target identity changed"));
        }
        checked(cancel, path)
    }
}

impl OriginalLink {
    fn capture(
        parent: &Arc<File>,
        parent_path: &Path,
        name: &OsStr,
        stub: Arc<File>,
        mut entry: TransferEntry,
        mount: Arc<MountAnchor>,
    ) -> Result<Self, FsError> {
        let revision = Revision::of(&stub, &entry.item.path)?;
        let payload = read_payload(&stub, &entry.item.path)?;
        entry.item.kind = EntryKind::DirectoryLink;
        Ok(Self {
            parent: parent.clone(),
            parent_path: parent_path.to_owned(),
            parent_identity: identity(parent, parent_path)?,
            mount,
            name: name.to_owned(),
            stub,
            entry,
            revision,
            payload,
        })
    }

    pub(super) fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    pub(super) fn name(&self) -> &OsStr {
        &self.name
    }

    pub(super) fn native_file(&self) -> Arc<File> {
        self.stub.clone()
    }

    pub(super) fn parent_mount(
        &self,
        path: &Path,
        identity: TransferIdentity,
    ) -> Result<Arc<MountAnchor>, FsError> {
        if self.parent_path != path || self.parent_identity != identity {
            return Err(invalid(
                path,
                "directory link mutation parent differs from its capture",
            ));
        }
        Ok(self.mount.clone())
    }

    pub(super) fn qualify_removal(&self) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        let stat = self
            .stub
            .metadata()
            .map_err(|failure| error(path, failure))?;
        if !stat.file_type().is_symlink()
            || stat.uid() != rustix::process::geteuid().as_raw()
            || stat.nlink() != 1
        {
            return Err(unsupported(
                path,
                "Directory link Move requires an owned symlink with one native name",
            ));
        }
        Ok(())
    }

    pub(super) fn verify_unlinked(&self) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        let stat = self
            .stub
            .metadata()
            .map_err(|failure| error(path, failure))?;
        if stat.nlink() != 0 {
            return Err(invalid(
                path,
                "the original link removal could not be confirmed",
            ));
        }
        Ok(())
    }

    pub(super) fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate_parent(cancel)?;
        self.verify_original(cancel)
    }

    pub(super) fn verify_original(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.verify_binding(
            &self.parent,
            &self.name,
            &self.entry.item.path,
            false,
            cancel,
        )
    }

    fn revalidate_parent(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        let visible_parent = self.mount.reopen(&self.parent_path, path)?;
        if identity(&visible_parent, path)? != self.parent_identity {
            return Err(invalid(path, "directory link parent changed"));
        }
        checked(cancel, path)
    }

    pub(super) fn verify_staged(
        &self,
        parent: &File,
        staged_path: &Path,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.verify_relocated(parent, &self.name, staged_path, cancel)
    }

    pub(super) fn verify_relocated(
        &self,
        parent: &File,
        name: &OsStr,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.verify_binding(parent, name, path, true, cancel)
    }

    fn verify_binding(
        &self,
        parent: &File,
        name: &OsStr,
        path: &Path,
        staged: bool,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        checked(cancel, path)?;
        let visible = File::from(
            openat2(
                parent,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE,
            )
            .map_err(|failure| error(path, failure.into()))?,
        );
        let metadata = visible.metadata().map_err(|failure| error(path, failure))?;
        let revision = Revision::of(&visible, path)?;
        let revision_matches = if staged {
            self.revision.after_rename_matches(revision)
        } else {
            self.revision == revision
        };
        if !metadata.file_type().is_symlink()
            || identity_of(&metadata) != self.entry.identity
            || !revision_matches
            || read_payload(&visible, path)? != self.payload
        {
            return Err(invalid(path, "directory link binding or payload changed"));
        }
        checked(cancel, path)
    }
}

fn read_payload(stub: &File, path: &Path) -> Result<OsString, FsError> {
    readlinkat(stub, "", Vec::new())
        .map(|payload| OsString::from_vec(payload.into_bytes()))
        .map_err(|failure| error(path, failure.into()))
}

fn open_target(parent: &File, name: &OsStr, path: &Path) -> Result<File, FsError> {
    match openat2(parent, name, PATH_DIRECTORY, Mode::empty(), LINK_RESOLVE) {
        Ok(target) => Ok(File::from(target)),
        Err(rustix::io::Errno::XDEV) => Err(unsupported(
            path,
            "directory link target crosses a mount boundary",
        )),
        Err(rustix::io::Errno::NOTDIR) => Err(unsupported(path, "link target is not a directory")),
        Err(failure) => Err(error(path, failure.into())),
    }
}

pub(super) struct MountAnchor {
    root: File,
    path: PathBuf,
    coverage: Vec<MountPoint>,
}

#[derive(PartialEq, Eq)]
struct MountPoint {
    id: u64,
    parent: u64,
    path: PathBuf,
}

impl MountAnchor {
    fn capture(parent: &File, path: &Path, cancel: &Cancellation) -> Result<Self, FsError> {
        let root = File::from(
            openat(CWD, "/", PATH_DIRECTORY, Mode::empty())
                .map_err(|failure| error(path, failure.into()))?,
        );
        let mount = native_location::mount(parent, path)?;
        let root_mount = native_location::mount(&root, path)?;
        if let (Some(mount), Some(root_mount)) = (mount, root_mount)
            && mount != root_mount
        {
            let points = mount_points(path)?;
            let point = points
                .iter()
                .find(|point| point.id == mount && path.starts_with(&point.path))
                .ok_or_else(|| unsupported(path, "directory link mount path is unqualified"))?;
            let mount_path = point.path.clone();
            let coverage = mount_coverage(points, &mount_path, mount)
                .ok_or_else(|| unsupported(path, "directory link mount lineage is unqualified"))?;
            return Ok(Self {
                root: pin_mount_root(parent, path, cancel)?,
                path: mount_path,
                coverage,
            });
        }
        // Without a mount ID, resolution from the namespace root still fails
        // closed on any separately mounted source.
        Ok(Self {
            root,
            path: PathBuf::from("/"),
            coverage: Vec::new(),
        })
    }

    pub(super) fn reopen(&self, parent_path: &Path, logical_path: &Path) -> Result<File, FsError> {
        let suffix = self.relative_parent(parent_path, logical_path)?;
        openat2(
            &self.root,
            suffix,
            PATH_DIRECTORY,
            Mode::empty(),
            LINK_RESOLVE,
        )
        .map(File::from)
        .map_err(|failure| error(logical_path, failure.into()))
    }

    pub(super) fn reopen_namespace(&self, path: &Path) -> Result<Option<File>, FsError> {
        let suffix = self.relative_parent(path, path)?;
        match openat2(
            &self.root,
            suffix,
            PATH_DIRECTORY,
            Mode::empty(),
            LINK_RESOLVE | ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(file) => Ok(Some(File::from(file))),
            Err(rustix::io::Errno::LOOP) => Ok(None),
            Err(failure) => Err(error(path, failure.into())),
        }
    }

    fn relative_parent<'path>(
        &self,
        parent_path: &'path Path,
        logical_path: &Path,
    ) -> Result<&'path Path, FsError> {
        let relative = parent_path.strip_prefix(&self.path).map_err(|_| {
            invalid(
                logical_path,
                "directory link parent left its captured mount",
            )
        })?;
        if let Some(mount) = self.coverage.last()
            && mount_coverage(mount_points(logical_path)?, &self.path, mount.id).as_ref()
                != Some(&self.coverage)
        {
            return Err(invalid(
                logical_path,
                "directory link mount lineage changed",
            ));
        }
        Ok(if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        })
    }
}

fn pin_mount_root(parent: &File, path: &Path, cancel: &Cancellation) -> Result<File, FsError> {
    let mut current = parent.try_clone().map_err(|failure| error(path, failure))?;
    for _ in 0..MAX_MOUNT_ANCESTORS {
        checked(cancel, path)?;
        match openat2(&current, "..", PATH_DIRECTORY, Mode::empty(), LINK_RESOLVE) {
            Ok(next) => current = File::from(next),
            Err(rustix::io::Errno::XDEV) => return Ok(current),
            Err(failure) => return Err(error(path, failure.into())),
        }
    }
    Err(unsupported(
        path,
        "directory link mount-root depth is unqualified",
    ))
}

fn mount_points(path: &Path) -> Result<Vec<MountPoint>, FsError> {
    let file = File::open("/proc/self/mountinfo").map_err(|failure| error(path, failure))?;
    let mut bytes = Vec::new();
    file.take(MAX_MOUNT_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|failure| error(path, failure))?;
    if bytes.len() as u64 > MAX_MOUNT_METADATA {
        return Err(unsupported(
            path,
            "directory link mount metadata exceeds its budget",
        ));
    }
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            parse_mount_point(line)
                .ok_or_else(|| unsupported(path, "directory link mount metadata is unqualified"))
        })
        .collect()
}

fn parse_mount_point(line: &[u8]) -> Option<MountPoint> {
    let mut fields = line.split(|byte| *byte == b' ');
    let id = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
    let parent = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
    let encoded = fields.nth(2)?;
    let mut raw = Vec::with_capacity(encoded.len());
    let mut suffix = encoded;
    while let Some((&byte, remaining)) = suffix.split_first() {
        suffix = remaining;
        if byte != b'\\' {
            raw.push(byte);
            continue;
        }
        raw.push(match suffix.get(..3)? {
            b"040" => b' ',
            b"011" => b'\t',
            b"012" => b'\n',
            b"134" => b'\\',
            _ => return None,
        });
        suffix = suffix.get(3..)?;
    }
    let path = PathBuf::from(OsString::from_vec(raw));
    path.is_absolute()
        .then_some(MountPoint { id, parent, path })
}

fn mount_coverage(mut points: Vec<MountPoint>, path: &Path, mount: u64) -> Option<Vec<MountPoint>> {
    points.retain(|point| path.starts_with(&point.path));
    points.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    // A stacked or disconnected prefix can hide the captured mount. Comparing
    // the visible lineage needs only namespace metadata and no foreign lookup.
    if points.last()?.id != mount
        || points
            .windows(2)
            .any(|pair| pair[0].path == pair[1].path || pair[0].id != pair[1].parent)
    {
        return None;
    }
    Some(points)
}

/// Metadata traversal starts a separate directory stream at each retained directory.
pub(super) fn discover(
    file: &Arc<File>,
    snapshot: &TransferEntry,
    traversal: &Traversal,
    empty: bool,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    let path = &snapshot.item.path;
    checked(cancel, path)?;
    if !matches!(snapshot.item.kind, EntryKind::Directory | EntryKind::File) {
        return Err(unsupported(
            path,
            "copying links and special entries is not supported",
        ));
    }
    visit(&snapshot.item);
    if snapshot.item.kind != EntryKind::Directory || empty {
        return Ok(());
    }
    traversal.revalidate(cancel)?;
    let directory = File::from(
        openat(&**file, ".", DIRECTORY, Mode::empty())
            .map_err(|failure| error(path, failure.into()))?,
    );
    let entries = Dir::read_from(&directory).map_err(|failure| error(path, failure.into()))?;
    for child in entries {
        checked(cancel, path)?;
        let child = child.map_err(|failure| error(path, failure.into()))?;
        let name = OsStr::from_bytes(child.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let mut child_traversal = traversal.clone();
        let child = probe(
            file,
            path,
            name,
            RESOLVE,
            Some(&mut child_traversal),
            cancel,
        )?;
        discover(
            &child.file,
            &child.entry,
            &child_traversal,
            child.projected_link.is_some() && child_traversal.empty(),
            cancel,
            visit,
        )?;
    }
    traversal.revalidate(cancel)
}
