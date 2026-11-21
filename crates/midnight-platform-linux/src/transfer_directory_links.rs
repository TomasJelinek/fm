//! Read-only Copy projections of qualified native directory links, followed file
//! links and preserved link leaves.

use super::destination::recovery::Revision;
use super::*;
use rustix::{
    fs::{Access, AtFlags, Dir, FileType, PROC_SUPER_MAGIC, accessat, fstat, fstatfs, readlinkat},
    io::Errno,
};
use std::{
    ffi::OsString,
    io::Read,
    os::{fd::AsRawFd, unix::ffi::OsStringExt},
    path::Component,
};

/// Most links a Copy follows inside one another. Ordinary folders do not count.
const MAX_LINK_DEPTH: usize = 128;
/// Most folders that metadata discovery opens inside one another; beyond it the
/// job copies without recursive totals, as with the traversal without links.
const MAX_DISCOVERY_DEPTH: usize = 128;
/// Most links that resolving one link's text follows, as the kernel allows.
const MAX_LINK_HOPS: usize = 40;
const MAX_MOUNT_ANCESTORS: usize = 4096;
const MAX_MOUNT_METADATA: u64 = 4 * 1024 * 1024;
const PATH_DIRECTORY: OFlags = OFlags::PATH.union(OFlags::DIRECTORY).union(OFlags::CLOEXEC);
const PATH_ANY: OFlags = OFlags::PATH.union(OFlags::CLOEXEC);
const PATH_LINK: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);
const LINK_RESOLVE: ResolveFlags = ResolveFlags::NO_MAGICLINKS.union(ResolveFlags::NO_XDEV);
/// A link's text that leaves the mount of its folder: resolved as the kernel resolves
/// it, across mount points, but never through a magic link.
const ACROSS_RESOLVE: ResolveFlags = ResolveFlags::NO_MAGICLINKS;
/// The kernel's interface filesystems by `statfs` f_type (linux/magic.h; the USB gadget
/// filesystems define theirs in drivers/usb/gadget: `FUNCTIONFS_MAGIC` in
/// function/f_fs.c, `GADGETFS_MAGIC` in legacy/inode.c), which no followed link leads
/// into: their files describe the system and the copying process itself
/// (/proc/self/environ is this process's environment), a read can wait for ever
/// (tracefs's trace_pipe, a gadget's endpoint files) or change state (xenfs's
/// hypervisor interfaces), and an automount point (autofs) would be mounted. Every other
/// filesystem holds files that Copy also reads when they are selected directly: local
/// ones, network ones (NFS, SMB, 9p) and FUSE. devtmpfs reports tmpfs's type: it holds
/// device nodes, which Copy never opens, and links, which follow these same rules.
const SYSTEM_FILESYSTEMS: [(u32, &str); 27] = [
    (0x9fa0, "proc"),
    (0x6265_6572, "sysfs"),
    (0x1cd1, "devpts"),
    (0x0027_e0eb, "cgroup"),
    (0x6367_7270, "cgroup2"),
    (0x6462_6720, "debugfs"),
    (0x7472_6163, "tracefs"),
    (0x7363_6673, "securityfs"),
    (0x5a3c_69f0, "apparmorfs"),
    (0x6165_676c, "pstore"),
    (0xcafe_4a11, "bpf"),
    (0x6265_6570, "configfs"),
    (0x6573_5543, "fusectl"),
    (0x1980_0202, "mqueue"),
    (0x0187, "autofs"),
    (0xde5e_81e4, "efivarfs"),
    (0x4249_4e4d, "binfmt_misc"),
    (0xf97c_ff8c, "selinuxfs"),
    (0x4341_5d53, "smackfs"),
    (0x6e73_6673, "nsfs"),
    (0x0765_5821, "resctrl"),
    (0x6c6f_6f70, "binderfs"),
    (0x6e66_7364, "nfsd"),
    (0x6759_6969, "rpc_pipefs"),
    (0xabba_1974, "xenfs"),
    (0x0a64_7361, "functionfs"),
    (0xaee7_1ee7, "gadgetfs"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectoryLinkMode {
    /// Directory links are followed; other links are refused.
    Follow,
    /// Directory links become empty directories; other links are refused.
    Empty,
    /// Every link is a leaf kept as a link; no target is opened.
    Preserve,
    /// Directory and regular-file links are followed; a link that cannot be
    /// followed (dangling, unreadable, special, on a system filesystem or a cycle)
    /// is a leaf.
    FollowLinks,
}

#[derive(Clone)]
pub(super) struct Traversal {
    mode: DirectoryLinkMode,
    directories: Option<Arc<Ancestor>>,
    links: Option<Arc<Binding>>,
}

/// A folder on the current path of a traversal that follows links, which a
/// followed link must not lead back to.
struct Ancestor {
    identity: TransferIdentity,
    /// Links followed into folders on the path, up to and including this one.
    links: usize,
    previous: Option<Arc<Self>>,
}

impl Drop for Ancestor {
    /// Ordinary folders have no depth limit, so a long path is released one
    /// folder after another rather than by one nested drop per folder.
    fn drop(&mut self) {
        let mut previous = self.previous.take();
        while let Some(ancestor) = previous {
            previous = match Arc::try_unwrap(ancestor) {
                Ok(mut ancestor) => ancestor.previous.take(),
                Err(_) => None,
            };
        }
    }
}

impl Traversal {
    pub(super) fn new(mode: DirectoryLinkMode) -> Self {
        Self {
            mode,
            directories: None,
            links: None,
        }
    }

    /// The traversal of an explicit link policy, or None for legacy Reject.
    pub(super) fn for_policy(links: CopyLinkPolicy) -> Option<Self> {
        let mode = match links {
            CopyLinkPolicy::Reject => return None,
            CopyLinkPolicy::Preserve => DirectoryLinkMode::Preserve,
            CopyLinkPolicy::FollowDirectoryContents => DirectoryLinkMode::Follow,
            CopyLinkPolicy::EmptyDirectories => DirectoryLinkMode::Empty,
            CopyLinkPolicy::FollowLinks => DirectoryLinkMode::FollowLinks,
        };
        Some(Self::new(mode))
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
                | (DirectoryLinkMode::Preserve, CopyLinkPolicy::Preserve)
                | (DirectoryLinkMode::FollowLinks, CopyLinkPolicy::FollowLinks)
        )
    }

    pub(super) fn has_bindings(&self) -> bool {
        self.links.is_some()
    }

    fn follows(&self) -> bool {
        matches!(
            self.mode,
            DirectoryLinkMode::Follow | DirectoryLinkMode::FollowLinks
        )
    }

    /// Whether this traversal yields link leaves (Preserve and FollowLinks).
    pub(super) fn recreates_links(&self) -> bool {
        matches!(
            self.mode,
            DirectoryLinkMode::Preserve | DirectoryLinkMode::FollowLinks
        )
    }

    fn is_active_ancestor(&self, identity: TransferIdentity) -> bool {
        let mut cursor = self.directories.as_deref();
        while let Some(ancestor) = cursor {
            if ancestor.identity == identity {
                return true;
            }
            cursor = ancestor.previous.as_deref();
        }
        false
    }

    /// The anchor of the folders below the innermost followed link, which a link
    /// met there shares as the anchor of its own folder (`Binding::below`).
    fn anchor(&self) -> Option<Anchor> {
        self.links.as_ref().and_then(Binding::below)
    }

    /// Records a folder the traversal enters. Only a traversal that follows
    /// links can reach a folder again on one path, so only it keeps the path's
    /// folders, refuses a followed link back to one of them and counts the
    /// links followed into folders against `MAX_LINK_DEPTH`. Ordinary folders,
    /// and every folder under Preserve or Empty, have no depth limit.
    fn enter_directory(
        &mut self,
        identity: TransferIdentity,
        path: &Path,
        projected_link: bool,
    ) -> Result<(), FsError> {
        if !self.follows() {
            return Ok(());
        }
        if projected_link && self.is_active_ancestor(identity) {
            return Err(invalid(path, "directory link target is an active ancestor"));
        }
        let links = self.directories.as_ref().map_or(0, |parent| parent.links)
            + usize::from(projected_link);
        if links > MAX_LINK_DEPTH {
            return Err(unsupported(
                path,
                "Linked Copy follows at most 128 links inside one another",
            ));
        }
        self.directories = Some(Arc::new(Ancestor {
            identity,
            links,
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
        self.anchor()
            .map(|anchor| anchor.reopen(parent_path, logical_path))
            .transpose()
    }
}

pub(super) struct Probe {
    pub(super) entry: TransferEntry,
    pub(super) file: Arc<File>,
    pub(super) projected_link: Option<Arc<Binding>>,
    /// A link kept as a link, never followed.
    pub(super) leaf: Option<Leaf>,
}

/// A link transferred as a link: authority over the link itself and, under
/// FollowLinks, why its target was not followed.
#[derive(Clone)]
pub(super) struct Leaf {
    pub(super) original: Arc<OriginalLink>,
    pub(super) unfollowed: Option<String>,
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
    let mut leaf = None;
    if let Some(traversal) = traversal {
        if snapshot.item.kind == EntryKind::Symlink {
            let followed = match traversal.mode {
                // The link itself is the whole transfer; its target is never opened.
                DirectoryLinkMode::Preserve => Followed::Not(None),
                DirectoryLinkMode::FollowLinks => follow(
                    parent,
                    parent_path,
                    name,
                    &file,
                    &snapshot,
                    traversal,
                    cancel,
                )?,
                DirectoryLinkMode::Follow | DirectoryLinkMode::Empty => {
                    Followed::Binding(Arc::new(project(
                        parent,
                        parent_path,
                        name,
                        &file,
                        &snapshot,
                        traversal,
                        cancel,
                    )?))
                }
            };
            match followed {
                Followed::Binding(binding) => {
                    snapshot = entry(&binding.target, &path)?;
                    if traversal.empty() {
                        snapshot.identity = binding.original_identity();
                    } else {
                        file = binding.target.clone();
                    }
                    traversal.links = Some(binding.clone());
                    projected_link = Some(binding);
                }
                Followed::Not(unfollowed) => {
                    leaf = Some(Leaf {
                        original: Arc::new(OriginalLink::leaf(
                            parent,
                            parent_path,
                            name,
                            file.clone(),
                            snapshot.clone(),
                        )?),
                        unfollowed,
                    });
                }
            }
        }
        if snapshot.item.kind == EntryKind::Directory {
            traversal.enter_directory(snapshot.identity, &path, projected_link.is_some())?;
        }
    }
    Ok(Probe {
        entry: snapshot,
        file,
        projected_link,
        leaf,
    })
}

enum Followed {
    Binding(Arc<Binding>),
    /// Kept as a link, with the reason FollowLinks did not follow it.
    Not(Option<String>),
}

/// FollowLinks: follow a directory or regular-file target that this process can
/// read, on any filesystem but the kernel's own (`SYSTEM_FILESYSTEMS`); any other
/// target keeps the link as a leaf. Errors reading the link itself, races and
/// cancellation stay errors.
fn follow(
    parent: &Arc<File>,
    parent_path: &Path,
    name: &OsStr,
    stub: &Arc<File>,
    snapshot: &TransferEntry,
    traversal: &Traversal,
    cancel: &Cancellation,
) -> Result<Followed, FsError> {
    let path = &snapshot.item.path;
    checked(cancel, path)?;
    let payload = read_payload(stub, path)?;
    let located = match locate(parent, parent_path, &payload, traversal, path, cancel) {
        Ok(Ok(located)) => located,
        // A magic link of /proc ends a resolution as a loop does. One can be met
        // across mounts, or within the mount of a folder in /proc.
        Ok(Err((Errno::LOOP, reach)))
            if reach == Reach::Across || system_filesystem(parent, path)? == Some("proc") =>
        {
            return Ok(Followed::Not(Some(
                "it points into a loop of links, or through a link of /proc to a process's \
                 open file or folder, which Copy never follows"
                    .into(),
            )));
        }
        Ok(Err((failure, _))) => return Ok(Followed::Not(Some(unfollowed_reason(failure)))),
        // Only a text the kernel does not resolve within the folder's mount (an
        // absolute one, say) needs that mount qualified to be walked; one that
        // cannot be (such as a folder reached through a link) keeps the link.
        Err(failure) if failure.kind != io::ErrorKind::Interrupted => {
            return Ok(Followed::Not(Some(format!(
                "the mount of its folder cannot be checked ({})",
                failure.message
            ))));
        }
        Err(failure) => return Err(failure),
    };
    if let Some(filesystem) = system_filesystem(&located.target, path)? {
        return Ok(Followed::Not(Some(system_reason(filesystem))));
    }
    let metadata = located
        .target
        .metadata()
        .map_err(|failure| error(path, failure))?;
    let kind = if metadata.is_dir() {
        TargetKind::Directory
    } else if metadata.is_file() {
        TargetKind::File
    } else {
        return Ok(Followed::Not(Some(
            "it points to a special file (a device, pipe or socket)".into(),
        )));
    };
    // The binding resolves the link again (a folder as a folder) and checks it
    // reaches the same target; the checks below are of what it holds.
    let binding = Binding::capture(
        parent,
        parent_path,
        name,
        stub.clone(),
        snapshot.clone(),
        traversal.links.clone(),
        located.mount,
        kind,
        cancel,
    )?;
    if kind == TargetKind::Directory && traversal.is_active_ancestor(binding.target_identity) {
        return Ok(Followed::Not(Some(
            "it points to a folder that contains it, so following it would never end".into(),
        )));
    }
    // The O_PATH opens needed only search permission on the way; copying needs
    // to read the file, or to list and enter the folder.
    if let Err(reason) = readable(&binding.target, kind) {
        return Ok(Followed::Not(Some(reason)));
    }
    Ok(Followed::Binding(Arc::new(binding)))
}

/// FollowDirectoryContents and EmptyDirectories: a directory link is projected
/// when it leads to a folder that FollowLinks would follow, and is an error
/// otherwise.
fn project(
    parent: &Arc<File>,
    parent_path: &Path,
    name: &OsStr,
    stub: &Arc<File>,
    snapshot: &TransferEntry,
    traversal: &Traversal,
    cancel: &Cancellation,
) -> Result<Binding, FsError> {
    let path = &snapshot.item.path;
    checked(cancel, path)?;
    let payload = read_payload(stub, path)?;
    let located = locate(parent, parent_path, &payload, traversal, path, cancel)?
        .map_err(|(failure, _)| error(path, failure.into()))?;
    if let Some(filesystem) = system_filesystem(&located.target, path)? {
        return Err(on_system_filesystem(path, filesystem));
    }
    if !located
        .target
        .metadata()
        .map_err(|failure| error(path, failure))?
        .is_dir()
    {
        return Err(unsupported(path, "link target is not a directory"));
    }
    Binding::capture(
        parent,
        parent_path,
        name,
        stub.clone(),
        snapshot.clone(),
        traversal.links.clone(),
        located.mount,
        TargetKind::Directory,
        cancel,
    )
}

/// Where a link's text leads from its folder (`locate`).
struct Located {
    /// The target, opened with O_PATH alone: not as a folder, so an automount
    /// point it names is not mounted.
    target: File,
    /// The mount of the outermost followed link's folder, when walking the text
    /// within it needed that mount captured.
    mount: Option<Arc<MountAnchor>>,
}

/// Where a link's `text` leads from `parent`, the link's folder: within the
/// folder's mount when the text stays there (`resolve_link_text`), otherwise as
/// the kernel resolves it across mount points, never through a magic link. The
/// inner error tells why the text leads nowhere and whether it was resolved
/// across mounts; the outer one is a failure to qualify the folder's mount.
fn locate(
    parent: &File,
    parent_path: &Path,
    text: &OsStr,
    traversal: &Traversal,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Result<Located, (Errno, Reach)>, FsError> {
    let anchor = traversal.anchor();
    let mut captured = None;
    let within = resolve_link_text(parent, text, false, path, false, || match &anchor {
        Some(anchor) => Ok(anchor.mount()),
        None => {
            let mount = Arc::new(MountAnchor::capture(parent, parent_path, cancel)?);
            captured = Some(mount.clone());
            Ok(Some(mount))
        }
    })?;
    let target = match within {
        Ok(target) => target,
        Err(Errno::XDEV) => match resolve_across(parent, text, false) {
            Ok(target) => target,
            Err(failure) => return Ok(Err((failure, Reach::Across))),
        },
        Err(failure) => return Ok(Err((failure, Reach::Within))),
    };
    Ok(Ok(Located {
        target,
        mount: captured,
    }))
}

/// Opens (O_PATH, following links) what a link's `text` names from `parent`,
/// the link's folder, without leaving the folder's mount. The kernel does it
/// under RESOLVE_NO_XDEV when it can. It cannot when the text is absolute: given
/// an absolute path, the kernel starts at `/` whatever mount `parent` is on (it
/// refuses only a jump to `/` met on the way, from another mount than the root's),
/// and an absolute link on the way of a folder that is not on the root's mount
/// crosses mount points back into it. Then the text is walked within the mount
/// (`MountAnchor::walk`, with `lineage_checked`), whose anchor `mount` supplies;
/// below a link followed to another mount there is none, and such a text leaves
/// the mount as far as the traversal can tell. The inner error tells why the
/// text does not resolve within the mount (EXDEV: it leaves the mount).
fn resolve_link_text(
    parent: &File,
    text: &OsStr,
    directory: bool,
    path: &Path,
    lineage_checked: bool,
    mount: impl FnOnce() -> Result<Option<Arc<MountAnchor>>, FsError>,
) -> Result<Result<File, Errno>, FsError> {
    let flags = if directory { PATH_DIRECTORY } else { PATH_ANY };
    let resolved = if Path::new(text).has_root() {
        Err(Errno::XDEV)
    } else {
        openat2(parent, text, flags, Mode::empty(), LINK_RESOLVE).map(File::from)
    };
    match resolved {
        Err(Errno::XDEV) => match mount()? {
            Some(mount) => mount.walk(parent, text, directory, path, lineage_checked),
            None => Ok(Err(Errno::XDEV)),
        },
        resolved => Ok(resolved),
    }
}

/// Opens (O_PATH) what a link's `text` names from `parent` as the kernel resolves
/// it, across mount points, but never through a magic link.
fn resolve_across(parent: &File, text: &OsStr, directory: bool) -> Result<File, Errno> {
    let flags = if directory { PATH_DIRECTORY } else { PATH_ANY };
    openat2(parent, text, flags, Mode::empty(), ACROSS_RESOLVE).map(File::from)
}

/// The name of the kernel interface filesystem (`SYSTEM_FILESYSTEMS`) that a
/// link's target lies on, or None for a filesystem that holds files.
fn system_filesystem(target: &File, path: &Path) -> Result<Option<&'static str>, FsError> {
    // The magic numbers are 32 bits wide; `f_type` is signed on most targets.
    let magic = fstatfs(target)
        .map_err(|failure| error(path, failure.into()))?
        .f_type as u32;
    Ok(system_filesystem_named(magic))
}

/// The name of the kernel interface filesystem whose `statfs` type is `magic`.
fn system_filesystem_named(magic: u32) -> Option<&'static str> {
    SYSTEM_FILESYSTEMS
        .iter()
        .find(|(known, _)| *known == magic)
        .map(|(_, name)| *name)
}

/// Why FollowLinks keeps a link to a target on a system filesystem, for the job details.
fn system_reason(filesystem: &str) -> String {
    if filesystem == "autofs" {
        "it points to an automount point that is not mounted (autofs), which Copy does \
         not mount"
            .into()
    } else {
        format!(
            "its target is on a system filesystem ({filesystem}), which Copy never follows \
             a link into"
        )
    }
}

/// A projected directory link whose target lies on a system filesystem.
fn on_system_filesystem(path: &Path, filesystem: &str) -> FsError {
    unsupported(
        path,
        &format!("the link target is on a system filesystem ({filesystem})"),
    )
}

/// Whether this process may read a followed target, checked on the held target with
/// the effective IDs; why not, in words for the job details, otherwise. A folder must
/// let it list and enter: opening `.` in it for reading needs both its read and search
/// permission, and needs no /proc. A file must let it read its content, which only
/// /proc's view of the held descriptor can check without opening the file again; where
/// /proc is not mounted the link is kept as a link and the details say so.
fn readable(target: &File, kind: TargetKind) -> Result<(), String> {
    match kind {
        TargetKind::Directory => openat(
            target,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(drop)
        .map_err(unfollowed_reason),
        TargetKind::File => {
            let descriptor = format!("/proc/self/fd/{}", target.as_raw_fd());
            match accessat(CWD, descriptor.as_str(), Access::READ_OK, AtFlags::EACCESS) {
                Ok(()) => Ok(()),
                Err(Errno::NOENT | Errno::NOTDIR) if !Path::new("/proc/self/fd").is_dir() => {
                    Err("its target's read permission cannot be checked without /proc".into())
                }
                Err(failure) => Err(unfollowed_reason(failure)),
            }
        }
    }
}

/// Why a link's target could not be opened, in words for the job details.
fn unfollowed_reason(failure: Errno) -> String {
    match failure {
        Errno::NOENT | Errno::NOTDIR => "its target does not exist".into(),
        Errno::LOOP => "it points to itself or into a loop of links".into(),
        Errno::ACCESS | Errno::PERM => "its target cannot be read (permission denied)".into(),
        failure => format!("its target cannot be opened ({})", io::Error::from(failure)),
    }
}

/// What a followed link must still point to when it is checked again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Directory,
    File,
}

/// How a followed link reaches its target from its folder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    /// Within the folder's mount: the folders below the target are opened again
    /// from the anchor of the link's folder, as that folder is.
    Within,
    /// By a way that leaves the folder's mount, to a target on another mount (or
    /// back on the same one): the link itself anchors the folders below it.
    Across,
}

/// Where a folder on the logical path of a traversal that follows links is opened
/// again, to check that the path still leads to it.
#[derive(Clone)]
enum Anchor {
    /// The pinned root of a captured mount: a logical path below the mount's path
    /// is resolved from it without leaving the mount (`MountAnchor::reopen`).
    Mount(Arc<MountAnchor>),
    /// A link followed across mounts: a logical path at or below the link is the
    /// link, resolved again to the same target, and the folders below that
    /// (`Binding::reopen_below`).
    Link(Arc<Binding>),
}

impl Anchor {
    fn reopen(&self, parent_path: &Path, logical_path: &Path) -> Result<File, FsError> {
        match self {
            Self::Mount(mount) => mount.reopen(parent_path, logical_path),
            Self::Link(binding) => binding.reopen_below(parent_path, logical_path),
        }
    }

    /// The mount whose root an absolute link text is walked from: none below a
    /// link followed across mounts.
    fn mount(&self) -> Option<Arc<MountAnchor>> {
        match self {
            Self::Mount(mount) => Some(mount.clone()),
            Self::Link(_) => None,
        }
    }
}

pub(super) struct Binding {
    original: Arc<OriginalLink>,
    target: Arc<File>,
    target_identity: TransferIdentity,
    target_kind: TargetKind,
    reach: Reach,
    previous: Option<Arc<Self>>,
}

/// Authority over the link itself. Binding owns the separate referent.
pub(super) struct OriginalLink {
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    /// Where a followed link's folder is opened again: the captured mount of the
    /// outermost followed link's folder, shared below links followed within it, or
    /// the nearest link followed across mounts above it. None for a link leaf,
    /// whose folder is checked by its source like any other entry's.
    anchor: Option<Anchor>,
    name: OsString,
    stub: Arc<File>,
    entry: TransferEntry,
    revision: Revision,
    payload: OsString,
}

impl Binding {
    /// `mount` is the captured mount of the link's folder when the caller has it.
    #[allow(clippy::too_many_arguments)]
    fn capture(
        parent: &Arc<File>,
        parent_path: &Path,
        name: &OsStr,
        stub: Arc<File>,
        snapshot: TransferEntry,
        previous: Option<Arc<Self>>,
        mount: Option<Arc<MountAnchor>>,
        target_kind: TargetKind,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        checked(cancel, &snapshot.item.path)?;
        let anchor = match mount
            .map(Anchor::Mount)
            .or_else(|| previous.as_ref().and_then(Self::below))
        {
            Some(anchor) => anchor,
            None => Anchor::Mount(Arc::new(MountAnchor::capture(parent, parent_path, cancel)?)),
        };
        let original = Arc::new(OriginalLink::capture(
            parent,
            parent_path,
            name,
            stub,
            snapshot,
            anchor,
        )?);
        // Resolve the captured payload exactly and independently check the alias
        // belongs to the same target through NO_MAGICLINKS. A folder is opened as a
        // folder, which mounts an automount point of its own filesystem (an NFS
        // submount); the target's filesystem is checked on what is held.
        let (target, reach) = open_target(&original, &original.payload, target_kind, false, None)?;
        let path = &original.entry.item.path;
        if let Some(filesystem) = system_filesystem(&target, path)? {
            return Err(on_system_filesystem(path, filesystem));
        }
        let binding = Self {
            target_identity: identity(&target, path)?,
            original,
            target: Arc::new(target),
            target_kind,
            reach,
            previous,
        };
        binding.revalidate_here(cancel)?;
        Ok(binding)
    }

    /// The anchor of the folders below this link, which a link met there shares as
    /// the anchor of its own folder: this link itself when it was followed across
    /// mounts, otherwise the anchor of its own folder, whose mount it stays on.
    fn below(self: &Arc<Self>) -> Option<Anchor> {
        match self.reach {
            Reach::Across => Some(Anchor::Link(self.clone())),
            Reach::Within => self.original.anchor.clone(),
        }
    }

    /// Opens again the folder at `parent_path`, which is this link's logical path
    /// or lies below it, for a link followed across mounts: the link, resolved
    /// again from its held folder, must still lead to its target, and the rest of
    /// the path is resolved from there without leaving the target's mount. Only
    /// links followed within that mount are met on the way.
    fn reopen_below(&self, parent_path: &Path, logical_path: &Path) -> Result<File, FsError> {
        let relative = parent_path
            .strip_prefix(&self.original.entry.item.path)
            .map_err(|_| invalid(logical_path, "directory link parent left its followed link"))?;
        let (target, _) = open_target(
            &self.original,
            &self.original.name,
            self.target_kind,
            true,
            Some(Reach::Across),
        )?;
        if identity(&target, logical_path)? != self.target_identity {
            return Err(invalid(logical_path, "link target identity changed"));
        }
        if relative.as_os_str().is_empty() {
            return Ok(target);
        }
        openat2(
            &target,
            relative,
            PATH_DIRECTORY,
            Mode::empty(),
            LINK_RESOLVE,
        )
        .map(File::from)
        .map_err(|failure| error(logical_path, failure.into()))
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
        // Through the link's name: the link as it is now, reached as it was
        // followed. Revalidating its folder checked the mount's lineage just now.
        let (target, _) = open_target(
            &self.original,
            &self.original.name,
            self.target_kind,
            true,
            Some(self.reach),
        )?;
        if identity(&target, path)? != self.target_identity {
            return Err(invalid(path, "link target identity changed"));
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
        anchor: Anchor,
    ) -> Result<Self, FsError> {
        let revision = Revision::of(&stub, &entry.item.path)?;
        let payload = read_payload(&stub, &entry.item.path)?;
        entry.item.kind = EntryKind::DirectoryLink;
        Ok(Self {
            parent: parent.clone(),
            parent_path: parent_path.to_owned(),
            parent_identity: identity(parent, parent_path)?,
            anchor: Some(anchor),
            name: name.to_owned(),
            stub,
            entry,
            revision,
            payload,
        })
    }

    /// A link kept as a link: its no-follow entry, revision and exact payload.
    /// The payload is checked against the representation budget here, so a link
    /// that cannot be represented fails before anything is staged.
    pub(super) fn leaf(
        parent: &Arc<File>,
        parent_path: &Path,
        name: &OsStr,
        stub: Arc<File>,
        entry: TransferEntry,
    ) -> Result<Self, FsError> {
        let revision = Revision::of(&stub, &entry.item.path)?;
        let payload = read_payload(&stub, &entry.item.path)?;
        midnight_core::TransferLinkRepresentation::linux_symbolic_link(payload.as_bytes())
            .map_err(|mut failure| {
                failure.path = entry.item.path.clone();
                failure
            })?;
        Ok(Self {
            parent: parent.clone(),
            parent_path: parent_path.to_owned(),
            parent_identity: identity(parent, parent_path)?,
            anchor: None,
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

    /// The exact link text captured at preparation.
    pub(super) fn payload(&self) -> &[u8] {
        self.payload.as_bytes()
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
        match &self.anchor {
            Some(Anchor::Mount(mount)) => Ok(mount.clone()),
            // Move never works below a followed link.
            Some(Anchor::Link(_)) => Err(invalid(
                path,
                "a link below a link followed to another mount has no captured mount",
            )),
            None => Err(invalid(path, "a preserved link has no captured mount")),
        }
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
                "Moving a link requires a link that you own and that has one name",
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
        let visible_parent = match &self.anchor {
            Some(anchor) => anchor.reopen(&self.parent_path, path)?,
            None => File::from(
                openat(CWD, &self.parent_path, DIRECTORY, Mode::empty())
                    .map_err(|failure| error(path, failure.into()))?,
            ),
        };
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

/// Opens the target of a followed link from the link's folder: `text` is the
/// captured payload, or the link's name, which follows the link. A text that stays
/// on the folder's mount is resolved within it (`resolve_link_text`), any other
/// across mount points (`resolve_across`). `reach` is that of a binding checked
/// again: a link followed within its mount must still stay there, and one followed
/// across mounts is resolved across them again.
fn open_target(
    original: &OriginalLink,
    text: &OsStr,
    kind: TargetKind,
    lineage_checked: bool,
    reach: Option<Reach>,
) -> Result<(File, Reach), FsError> {
    let path = &original.entry.item.path;
    let directory = kind == TargetKind::Directory;
    let within = if reach == Some(Reach::Across) {
        Err(Errno::XDEV)
    } else {
        resolve_link_text(
            &original.parent,
            text,
            directory,
            path,
            lineage_checked,
            || Ok(original.anchor.as_ref().and_then(Anchor::mount)),
        )?
    };
    let resolved = match within {
        Ok(target) => Ok((target, Reach::Within)),
        Err(Errno::XDEV) if reach != Some(Reach::Within) => {
            resolve_across(&original.parent, text, directory).map(|target| (target, Reach::Across))
        }
        Err(failure) => Err(failure),
    };
    let (target, reach) = match resolved {
        Ok(resolved) => resolved,
        Err(Errno::XDEV) => {
            return Err(unsupported(
                path,
                "directory link target left the mount it was followed within",
            ));
        }
        Err(Errno::NOTDIR) => {
            return Err(unsupported(path, "link target is not a directory"));
        }
        Err(failure) => return Err(error(path, failure.into())),
    };
    if kind == TargetKind::File
        && !target
            .metadata()
            .map_err(|failure| error(path, failure))?
            .is_file()
    {
        return Err(invalid(path, "link target is no longer a regular file"));
    }
    Ok((target, reach))
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
        match openat2(
            &self.root,
            suffix,
            PATH_DIRECTORY,
            Mode::empty(),
            LINK_RESOLVE,
        ) {
            Ok(file) => Ok(File::from(file)),
            // A followed link with an absolute text on the way; the lineage
            // was checked just now.
            Err(Errno::XDEV) => self
                .walk(&self.root, suffix.as_os_str(), true, logical_path, true)?
                .map_err(|failure| error(logical_path, failure.into())),
            Err(failure) => Err(error(logical_path, failure.into())),
        }
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
        self.check_lineage(logical_path)?;
        Ok(if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        })
    }

    /// The mounts along this mount's path are still those of its capture, so
    /// the path still leads to this mount.
    fn check_lineage(&self, logical_path: &Path) -> Result<(), FsError> {
        if let Some(mount) = self.coverage.last()
            && mount_coverage(mount_points(logical_path)?, &self.path, mount.id).as_ref()
                != Some(&self.coverage)
        {
            return Err(invalid(
                logical_path,
                "directory link mount lineage changed",
            ));
        }
        Ok(())
    }

    /// Opens (O_PATH) what a link's `text` names from `start`, a folder on this
    /// mount, as the kernel resolves it but without leaving the mount. Each name
    /// is opened alone, not followed, under RESOLVE_NO_XDEV, so a mount point on
    /// the way and `..` above the mount's root fail with EXDEV. A link met on the
    /// way is read: its text continues from the link's folder, or from the
    /// mount's root when it is absolute and lies under the mount's path (EXDEV
    /// otherwise), once the mount's lineage is checked (unless
    /// `lineage_checked`). A procfs (magic) link and more than `MAX_LINK_HOPS`
    /// links fail with ELOOP, as with the kernel. `directory` requires a folder.
    /// The inner error tells why the text does not resolve; the outer one is a
    /// failure to check the lineage.
    fn walk(
        &self,
        start: &File,
        text: &OsStr,
        directory: bool,
        logical_path: &Path,
        lineage_checked: bool,
    ) -> Result<Result<File, Errno>, FsError> {
        let walk = Walk {
            mount: self,
            logical_path,
            lineage_checked,
            current: start
                .try_clone()
                .map_err(|failure| error(logical_path, failure))?,
            pending: Vec::new(),
        };
        match walk.resolve(text, directory) {
            Ok(target) => Ok(Ok(target)),
            Err(Unresolved::Text(failure)) => Ok(Err(failure)),
            Err(Unresolved::Mount(failure)) => Err(failure),
        }
    }
}

/// One resolution of a link's text within a mount (`MountAnchor::walk`).
struct Walk<'a> {
    mount: &'a MountAnchor,
    logical_path: &'a Path,
    lineage_checked: bool,
    /// The folder in which the next name is opened.
    current: File,
    /// The names still to open, the next one last.
    pending: Vec<OsString>,
}

/// Why a walk ended without a target.
enum Unresolved {
    /// The text does not resolve within the mount, as the kernel would say.
    Text(Errno),
    /// The mount could not be checked.
    Mount(FsError),
}

impl From<Errno> for Unresolved {
    fn from(failure: Errno) -> Self {
        Self::Text(failure)
    }
}

impl Walk<'_> {
    fn resolve(mut self, text: &OsStr, directory: bool) -> Result<File, Unresolved> {
        self.queue(text)?;
        let mut hops = 0;
        while let Some(name) = self.pending.pop() {
            let next = File::from(openat2(
                &self.current,
                &name,
                PATH_LINK,
                Mode::empty(),
                LINK_RESOLVE,
            )?);
            if FileType::from_raw_mode(fstat(&next)?.st_mode) != FileType::Symlink {
                self.current = next;
                continue;
            }
            hops += 1;
            if hops > MAX_LINK_HOPS || fstatfs(&next)?.f_type == PROC_SUPER_MAGIC {
                return Err(Errno::LOOP.into());
            }
            let text = OsString::from_vec(readlinkat(&next, "", Vec::new())?.into_bytes());
            self.queue(&text)?;
        }
        if directory
            && FileType::from_raw_mode(fstat(&self.current)?.st_mode) != FileType::Directory
        {
            return Err(Errno::NOTDIR.into());
        }
        Ok(self.current)
    }

    /// Queues the names of `text` before the remaining ones. An absolute text
    /// starts again from the mount's root.
    fn queue(&mut self, text: &OsStr) -> Result<(), Unresolved> {
        if text.is_empty() {
            return Err(Errno::NOENT.into());
        }
        let mut names = Path::new(text);
        if names.has_root() {
            names = names
                .strip_prefix(&self.mount.path)
                .map_err(|_| Errno::XDEV)?;
            if !self.lineage_checked {
                self.mount
                    .check_lineage(self.logical_path)
                    .map_err(Unresolved::Mount)?;
                self.lineage_checked = true;
            }
            self.current = self
                .mount
                .root
                .try_clone()
                .map_err(|failure| Unresolved::Mount(error(self.logical_path, failure)))?;
        }
        // A text that ends in `/` or `/.` names a folder, which components
        // forget: opening `.` in what it names last checks that.
        if text.as_bytes().ends_with(b"/") || text.as_bytes().ends_with(b"/.") {
            self.pending.push(".".into());
        }
        let start = self.pending.len();
        for name in names.components() {
            match name {
                Component::Normal(name) => self.pending.push(name.to_owned()),
                Component::ParentDir => self.pending.push("..".into()),
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
        self.pending[start..].reverse();
        Ok(())
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

/// Metadata traversal starts a separate directory stream at each retained
/// directory, at most `MAX_DISCOVERY_DEPTH` inside one another.
pub(super) fn discover(
    file: &Arc<File>,
    snapshot: &TransferEntry,
    traversal: &Traversal,
    empty: bool,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    discover_at(file, snapshot, traversal, empty, 1, cancel, visit)
}

fn discover_at(
    file: &Arc<File>,
    snapshot: &TransferEntry,
    traversal: &Traversal,
    empty: bool,
    depth: usize,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    let path = &snapshot.item.path;
    checked(cancel, path)?;
    // A link leaf is transferred as a link: count it, never open its target.
    if snapshot.item.kind == EntryKind::Symlink && traversal.recreates_links() {
        visit(&snapshot.item);
        return Ok(());
    }
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
    if depth > MAX_DISCOVERY_DEPTH {
        return Err(unsupported(
            path,
            "Discovery exceeds the 128-directory depth budget",
        ));
    }
    // The folders discovery is inside hold their descriptors meanwhile.
    let _level = descriptor_budget::levels().enter(path)?;
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
        discover_at(
            &child.file,
            &child.entry,
            &child_traversal,
            child.projected_link.is_some() && child_traversal.empty(),
            depth + 1,
            cancel,
            visit,
        )?;
    }
    traversal.revalidate(cancel)
}

#[cfg(test)]
#[path = "transfer_directory_link_walk_tests.rs"]
mod walk_tests;

#[cfg(test)]
#[path = "transfer_directory_link_filesystem_tests.rs"]
mod filesystem_tests;
