//! Read-only metadata traversal through pinned directory identities, without following links.
use super::*;
use rustix::fs::Dir;
use std::os::unix::fs::MetadataExt;

struct Frame {
    directory: Arc<File>,
    entries: Dir,
    path: PathBuf,
}
pub(crate) fn walk(
    parent: &Arc<File>,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    check_cancel(cancel, path)?;
    let mut stack = Vec::new();
    if let Some(frame) = inspect(parent, source, name, path, cancel, visit)? {
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
        let source = File::from(
            openat2(
                &*frame.directory,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|error| native_error(&path, error))?,
        );
        if let Some(child) = inspect(&frame.directory, &source, name, &path, cancel, visit)? {
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
    parent: &Arc<File>,
    source: &File,
    name: &OsStr,
    path: &Path,
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
    let directory = File::from(
        openat2(
            &**parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|error| native_error(path, error))?,
    );
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
