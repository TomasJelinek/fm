//! Read-only, handle-relative Find sources. No implicit links or child mount crossings.
//!
//! A chosen location is resolved like panel navigation: ordinary symbolic links in
//! its ancestors are followed (a merged-/usr `/lib/modules` or a folder entered
//! through a directory link), procfs magic links are not, and the final component
//! must not be a link. Descendants are opened relative to their pinned parent
//! without links or mount crossings. Each directory's visible binding is checked
//! once when its enumeration starts; entries created, removed or renamed inside it
//! afterwards do not abort the search. Every result is revalidated by its own path.
use midnight_core::*;
use rustix::fs::{CWD, Dir, Mode, OFlags, ResolveFlags, openat, openat2};
use std::{
    fs::File,
    io::{self, Read},
    os::unix::{ffi::OsStrExt, fs::MetadataExt, io::AsRawFd},
    path::Path,
};

const CHILD_RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_XDEV);
/// Absolute locations: ancestor symlinks are followed, magic links are refused.
const PATH_RESOLVE: ResolveFlags = ResolveFlags::NO_MAGICLINKS;
const PROBE: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

impl FindService for crate::LinuxFileSystem {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        checked(cancel, path)?;
        if !path.is_absolute()
            || path
                .as_os_str()
                .as_bytes()
                .split(|b| *b == b'/')
                .any(|part| part == b"." || part == b"..")
        {
            return Err(find_error(
                path,
                io::ErrorKind::InvalidInput,
                "An absolute native path without dot steps is required.",
            ));
        }
        let probe = File::from(
            openat2(CWD, path, PROBE, Mode::empty(), PATH_RESOLVE)
                .map_err(|error| failure(path, error))?,
        );
        let entry = snapshot(&probe, path)?;
        ordinary(&entry)?;
        let file = if entry.kind == EntryKind::Directory {
            File::from(
                openat2(CWD, path, DIRECTORY, Mode::empty(), PATH_RESOLVE)
                    .map_err(|error| failure(path, error))?,
            )
        } else {
            probe
        };
        if !snapshot(&file, path)?.matches_capture(&entry) {
            return Err(changed(path));
        }
        Ok(Box::new(Source {
            entry,
            file,
            children: None,
            reader: None,
        }))
    }
}

struct Source {
    entry: FindEntry,
    file: File,
    children: Option<Dir>,
    reader: Option<File>,
}
impl Source {
    fn child(&self, name: &std::ffi::OsStr) -> Result<FindChild, FsError> {
        let path = self.entry.path.join(name);
        let probe = match openat2(&self.file, name, PROBE, Mode::empty(), CHILD_RESOLVE) {
            Ok(file) => File::from(file),
            Err(error) if error == rustix::io::Errno::XDEV => {
                return Ok(FindChild::Skipped {
                    path,
                    reason: FindSkipReason::MountBoundary,
                });
            }
            Err(error) => return Err(failure(&path, error)),
        };
        let entry = snapshot(&probe, &path)?;
        if entry.kind == EntryKind::Symlink {
            return Ok(FindChild::Skipped {
                path,
                reason: FindSkipReason::Link,
            });
        }
        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
            return Ok(FindChild::Skipped {
                path,
                reason: FindSkipReason::Special,
            });
        }
        let file = if entry.kind == EntryKind::Directory {
            File::from(
                openat2(&self.file, name, DIRECTORY, Mode::empty(), CHILD_RESOLVE)
                    .map_err(|error| failure(&path, error))?,
            )
        } else {
            probe
        };
        if !snapshot(&file, &path)?.matches_capture(&entry) {
            return Err(changed(&path));
        }
        Ok(FindChild::Entry(Box::new(Source {
            entry,
            file,
            children: None,
            reader: None,
        })))
    }
}
impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    /// Same device, inode, size, mtime and mode; a ctime that moved alone (xattr, chown or
    /// link-count changes) still counts as the captured file
    /// (`FindEntry::matches_capture_except_change_time`).
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.path)?;
        let visible = File::from(
            openat2(CWD, &self.entry.path, PROBE, Mode::empty(), PATH_RESOLVE)
                .map_err(|error| failure(&self.entry.path, error))?,
        );
        if !snapshot(&visible, &self.entry.path)?.matches_capture_except_change_time(&self.entry)
            || !snapshot(&self.file, &self.entry.path)?
                .matches_capture_except_change_time(&self.entry)
        {
            return Err(changed(&self.entry.path));
        }
        Ok(())
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        checked(cancel, &self.entry.path)?;
        if self.entry.kind != EntryKind::Directory {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "Find source is not a directory.",
            ));
        }
        if self.children.is_none() {
            // The visible binding is checked once; the pinned directory is then
            // enumerated even while entries inside it change.
            self.revalidate(cancel)?;
            self.children =
                Some(Dir::read_from(&self.file).map_err(|error| failure(&self.entry.path, error))?);
        }
        loop {
            checked(cancel, &self.entry.path)?;
            let Some(next) = self.children.as_mut().and_then(Iterator::next) else {
                return Ok(None);
            };
            let next = next.map_err(|error| failure(&self.entry.path, error))?;
            let name = std::ffi::OsStr::from_bytes(next.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            return Ok(Some(match self.child(name) {
                Ok(child) => child,
                Err(error) => FindChild::Failed(error),
            }));
        }
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        checked(cancel, &self.entry.path)?;
        if self.entry.kind != EntryKind::File {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "Directory content is not a byte stream.",
            ));
        }
        if self.reader.is_none() {
            // Reopen the pinned O_PATH object, never a potentially substituted device name.
            let descriptor = format!("/proc/self/fd/{}", self.file.as_raw_fd());
            let reader = File::from(
                openat(
                    CWD,
                    descriptor.as_str(),
                    OFlags::RDONLY
                        .union(OFlags::NONBLOCK)
                        .union(OFlags::CLOEXEC),
                    Mode::empty(),
                )
                .map_err(|error| failure(&self.entry.path, error))?,
            );
            if !snapshot(&reader, &self.entry.path)?.matches_capture_except_change_time(&self.entry)
            {
                return Err(changed(&self.entry.path));
            }
            self.reader = Some(reader);
        }
        match &mut self.reader {
            Some(reader) => reader
                .read(buffer)
                .map_err(|error| FsError::from_io("find content", &self.entry.path, error)),
            None => Err(find_error(
                &self.entry.path,
                io::ErrorKind::Other,
                "Find content reader is unavailable.",
            )),
        }
    }
}

fn ordinary(entry: &FindEntry) -> Result<(), FsError> {
    if matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
        Ok(())
    } else {
        Err(find_error(
            &entry.path,
            io::ErrorKind::Unsupported,
            "Find roots/results must be ordinary native files or directories; links are not followed.",
        ))
    }
}
/// Captures a node's facts. Its localized modification time is left to the engine, which
/// formats only the matches it keeps: most examined nodes never match.
fn snapshot(file: &File, path: &Path) -> Result<FindEntry, FsError> {
    let metadata = file
        .metadata()
        .map_err(|error| FsError::from_io("find metadata", path, error))?;
    let kind = if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    };
    let mut object = [0; 16];
    object[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    // A directory's size and times change with every entry created, removed or
    // renamed inside it. Enumeration uses the pinned handle and result navigation
    // needs only its identity, so directory revisions carry no content state.
    let revision = if kind == EntryKind::Directory {
        FindRevision([0; 6])
    } else {
        FindRevision([
            metadata.len(),
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
            metadata.mode() as u64,
        ])
    };
    Ok(FindEntry {
        path: path.to_owned(),
        kind,
        identity: TransferIdentity {
            device: metadata.dev(),
            object,
        },
        revision,
        metadata: EntryMetadata {
            size: (kind == EntryKind::File).then_some(metadata.len()),
            modified: metadata.modified().ok(),
            unix_mode: Some(metadata.mode()),
            ..Default::default()
        },
    })
}
fn checked(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(find_error(
            path,
            io::ErrorKind::Interrupted,
            "Find cancelled.",
        ))
    } else {
        Ok(())
    }
}
fn changed(path: &Path) -> FsError {
    find_error(
        path,
        io::ErrorKind::InvalidData,
        "Find object identity or revision changed.",
    )
}
fn failure(path: &Path, error: rustix::io::Errno) -> FsError {
    FsError::from_io("find files", path, error.into())
}
