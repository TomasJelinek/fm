//! Read-only metadata traversal through pinned directory identities, without following links.
//! Mount points below the selected root are reported as unsupported entries, not entered.
use super::*;
use rustix::{fd::OwnedFd, fs::Dir};
use std::os::unix::fs::MetadataExt;

const CHILD: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_XDEV)
    .union(ResolveFlags::NO_SYMLINKS);

struct Frame {
    directory: Arc<File>,
    entries: Dir,
    path: PathBuf,
}
/// Visits `source` (pinned by the caller, which revalidates its binding) and its
/// descendants. The root is opened through its pinned handle, so a selected root
/// that is itself a mount point can be enumerated.
pub(crate) fn walk(
    source: &File,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    check_cancel(cancel, path)?;
    let mut stack = Vec::new();
    let root = || {
        openat(
            source,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
    };
    if let Some(frame) = inspect(source, path, root, cancel, visit)? {
        stack.push(frame);
    }
    while let Some(frame) = stack.last_mut() {
        check_cancel(cancel, &frame.path)?;
        let Some(next) = frame.entries.next() else {
            stack.pop();
            continue;
        };
        let next = next.map_err(|error| native_error(&frame.path, error))?;
        let name = OsStr::from_bytes(next.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let path = frame.path.join(name);
        let source = match openat2(
            &*frame.directory,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            CHILD,
        ) {
            Ok(source) => File::from(source),
            Err(rustix::io::Errno::XDEV) => {
                // A nested mount point is never entered; the operation itself
                // reports it as unsupported. Discovery continues with its siblings.
                visit(&MutationItem {
                    path,
                    kind: EntryKind::Other,
                    metadata: EntryMetadata::default(),
                });
                continue;
            }
            Err(error) => return Err(native_error(&path, error)),
        };
        let directory = frame.directory.clone();
        let child = || {
            openat2(
                &*directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                CHILD,
            )
        };
        if let Some(child) = inspect(&source, &path, child, cancel, visit)? {
            if stack.len() >= 128 {
                return Err(error(
                    &path,
                    ErrorKind::Unsupported,
                    "Discovery exceeds the 128-directory depth budget",
                ));
            }
            stack.push(child);
        }
    }
    Ok(())
}
fn inspect(
    source: &File,
    path: &Path,
    open_directory: impl FnOnce() -> rustix::io::Result<OwnedFd>,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<Option<Frame>, FsError> {
    check_cancel(cancel, path)?;
    let metadata = source
        .metadata()
        .map_err(|error| FsError::from_io("discover metadata", path, error))?;
    let kind = if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    visit(&MutationItem {
        path: path.to_owned(),
        kind,
        metadata: EntryMetadata {
            size: (kind == EntryKind::File).then_some(metadata.len()),
            modified: metadata.modified().ok(),
            unix_mode: Some(metadata.mode()),
            ..EntryMetadata::default()
        },
    });
    if kind != EntryKind::Directory {
        return Ok(None);
    }
    let directory = File::from(open_directory().map_err(|error| native_error(path, error))?);
    if Identity::of(&directory, path)? != Identity::of(source, path)? {
        return Err(error(
            path,
            ErrorKind::InvalidInput,
            "Discovery directory identity changed",
        ));
    }
    let entries = Dir::read_from(&directory).map_err(|error| native_error(path, error))?;
    Ok(Some(Frame {
        directory: Arc::new(directory),
        entries,
        path: path.to_owned(),
    }))
}
