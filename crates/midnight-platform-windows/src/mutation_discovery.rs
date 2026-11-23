//! Read-only, handle-relative discovery. Children are classified, sized and timed from
//! directory enumeration records; only directories are opened, once, to list them.
use super::*;
use crate::{directory_records::Cursor, reparse::Native};
use std::{collections::VecDeque, os::windows::ffi::OsStringExt, time::SystemTime};

struct Child {
    name: OsString,
    native: Native,
    size: u64,
    modified: Option<SystemTime>,
}
struct Frame {
    directory: File,
    path: PathBuf,
    children: VecDeque<Child>,
    cursor: Cursor,
}
/// The folder depth that the discovery of a mutation other than a transfer stops at.
const MUTATION_DEPTH: usize = 128;

pub(crate) fn walk(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    walk_within(
        parent,
        source,
        name,
        path,
        Some(MUTATION_DEPTH),
        cancel,
        visit,
    )
}

/// The discovery of a Copy or Move: as deep as the transfer itself goes, which keeps no
/// fixed depth limit on Windows (each level holds one listing handle meanwhile).
pub(crate) fn walk_transfer(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    walk_within(parent, source, name, path, None, cancel, visit)
}

fn walk_within(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    max_depth: Option<usize>,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    checked(cancel, path)?;
    let mut stack = Vec::new();
    if let Some(frame) = inspect_root(parent, source, name, path, cancel, visit)? {
        stack.push(frame);
    }
    loop {
        let depth = stack.len();
        let Some(frame) = stack.last_mut() else {
            return Ok(());
        };
        checked(cancel, &frame.path)?;
        if frame.children.is_empty() && !frame.cursor.ended() {
            fill(frame)?;
        }
        let Some(child) = frame.children.pop_front() else {
            if frame.cursor.ended() {
                stack.pop();
            }
            continue;
        };
        let path = frame.path.join(&child.name);
        let kind = child.native.kind();
        visit(&MutationItem {
            path: path.clone(),
            kind,
            metadata: EntryMetadata {
                size: (kind == EntryKind::File).then_some(child.size),
                modified: child.modified,
                native_attributes: Some(child.native.attributes),
                ..EntryMetadata::default()
            },
        });
        if kind != EntryKind::Directory {
            continue;
        }
        if max_depth.is_some_and(|limit| depth >= limit) {
            return Err(failure(
                "discover",
                &path,
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Discovery exceeds the 128-directory depth budget",
                ),
            ));
        }
        // The listing handle never follows a link; a child replaced by a link after
        // enumeration is refused rather than traversed.
        let directory = native::open_listing(&frame.directory, &child.name)
            .map_err(|error| failure("discover directory", &path, error))?;
        stack.push(Frame {
            directory,
            path,
            children: VecDeque::new(),
            cursor: Cursor::default(),
        });
    }
}
fn inspect_root(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<Option<Frame>, FsError> {
    checked(cancel, path)?;
    let metadata = source
        .metadata()
        .map_err(|error| failure("discover metadata", path, error))?;
    let native = Native::of(source, metadata.file_attributes())
        .map_err(|error| failure("discover metadata", path, error))?;
    let kind = native.kind();
    visit(&MutationItem {
        path: path.to_owned(),
        kind,
        metadata: EntryMetadata {
            size: (kind == EntryKind::File).then_some(metadata.file_size()),
            modified: metadata.modified().ok(),
            native_attributes: Some(native.attributes),
            ..EntryMetadata::default()
        },
    });
    if kind != EntryKind::Directory {
        return Ok(None);
    }
    let directory = native::open_enumeration(parent, source, name)
        .map_err(|error| failure("discover directory", path, error))?;
    Ok(Some(Frame {
        directory,
        path: path.to_owned(),
        children: VecDeque::new(),
        cursor: Cursor::default(),
    }))
}
fn fill(frame: &mut Frame) -> Result<(), FsError> {
    let children = &mut frame.children;
    frame
        .cursor
        .next_batch(&frame.directory, &mut |record| {
            children.push_back(Child {
                name: OsString::from_wide(record.name),
                native: record.native,
                size: record.size,
                modified: record.modified(),
            });
            Ok(())
        })
        .map_err(|error| failure("discover directory", &frame.path, error))
}
