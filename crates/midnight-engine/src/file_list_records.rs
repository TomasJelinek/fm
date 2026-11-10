//! Backend selection expansion and cached metadata for Make File List.
use midnight_core::*;
use std::{collections::BTreeMap, io, path::Path};

#[cfg(windows)]
#[path = "file_list_records_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "file_list_records_linux.rs"]
mod native;

pub(super) fn error(message: impl Into<String>) -> FsError {
    FsError::from_io(
        "make file list",
        Path::new(""),
        io::Error::other(message.into()),
    )
}

pub(super) fn check(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error("File list generation was cancelled."))
    } else {
        Ok(())
    }
}

pub(super) fn selected<'a>(
    capture: &'a FileListCapture,
    cancel: &Cancellation,
) -> Result<Vec<&'a Entry>, FsError> {
    let mut rows = Vec::new();
    match &capture.source {
        FileListSource::Native {
            entries,
            view,
            selection,
            fallback,
        } => {
            for entry in entries.iter() {
                check(cancel)?;
                let hit = if selection.is_empty() {
                    *fallback == Some(entry.id)
                } else {
                    entry
                        .native_path()
                        .is_some_and(|path| selection.contains_path(path))
                };
                if hit && let Some(position) = view.by_id.get(&entry.id) {
                    rows.push((*position, entry));
                }
            }
        }
        FileListSource::Archive {
            entries,
            view,
            selected,
            fallback,
        } => {
            for entry in entries.iter() {
                check(cancel)?;
                let hit = if selected.is_empty() {
                    *fallback == Some(entry.id)
                } else {
                    selected.contains(&entry.id)
                };
                if hit && let Some(position) = view.by_id.get(&entry.id) {
                    rows.push((*position, entry));
                }
            }
        }
        FileListSource::Tree {
            view,
            selection,
            fallback,
        } => {
            for (position, row) in view.rows.iter().enumerate() {
                check(cancel)?;
                if row.selectable
                    && !row.context_only
                    && if selection.is_empty() {
                        *fallback == Some(row.id)
                    } else {
                        selection.contains_row(row.id)
                    }
                {
                    rows.push((position, &row.entry));
                }
            }
        }
    }
    rows.sort_unstable_by_key(|(position, _)| *position);
    let result: Vec<_> = rows
        .into_iter()
        .map(|(_, entry)| entry)
        .filter(|entry| entry.name != "." && entry.name != "..")
        .collect();
    if result.is_empty() {
        return Err(error("No displayed files or folders are selected."));
    }
    Ok(result)
}

pub(super) fn record(
    entry: &Entry,
    directory: &str,
    environment: &BTreeMap<String, String>,
) -> Result<FileListRecord, FsError> {
    let name = entry
        .name
        .to_str()
        .ok_or_else(|| error("A filename cannot be represented losslessly in Unicode text."))?;
    let is_dir = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
    let (stem, extension) = if is_dir {
        (name, "")
    } else {
        name.rsplit_once('.')
            .filter(|(_, extension)| !extension.is_empty())
            .unwrap_or((name, ""))
    };
    let size = if is_dir {
        // A directory record's own storage length is not a recursive total.
        // The worker substitutes only completed, error-free captured totals.
        "<DIR>".into()
    } else {
        entry
            .metadata
            .size
            .map_or_else(|| "-".into(), |value| value.to_string())
    };
    let prefix = match entry.native_path().and_then(Path::parent) {
        Some(path) => path.to_str().ok_or_else(|| {
            error("A directory cannot be represented losslessly in Unicode text.")
        })?,
        None => directory,
    };
    let (drive, path) = native::parts(prefix);
    Ok(FileListRecord {
        file_name: name.into(),
        name_part: stem.into(),
        extension: extension.into(),
        size,
        date: entry
            .metadata
            .localized_modified
            .as_ref()
            .map_or("-", |value| &value.date)
            .into(),
        time: entry
            .metadata
            .localized_modified
            .as_ref()
            .map_or("-", |value| &value.time)
            .into(),
        attributes: native::attributes(&entry.metadata),
        dos_name: "-".into(),
        drive,
        dos_path: path.clone(),
        path,
        environment: environment.clone(),
    })
}

pub(super) fn captured_record(
    entry: &Entry,
    capture: &FileListCapture,
    environment: &BTreeMap<String, String>,
    cancel: &Cancellation,
) -> Result<FileListRecord, FsError> {
    check(cancel)?;
    let FileListSource::Tree { view, .. } = &capture.source else {
        return record(entry, &capture.directory, environment);
    };
    if entry.virtual_node.is_none() {
        return record(entry, &capture.directory, environment);
    }
    // Entry IDs identify displayed occurrences, including duplicate names in
    // distinct branches. Never reconstruct a virtual path from a host PathBuf
    // or from provider entry IDs that could occur in more than one branch.
    let mut position = *view
        .index
        .directory
        .by_id
        .get(&entry.id)
        .ok_or_else(|| error("The captured tree entry is no longer in its index."))?;
    let row = view
        .rows
        .get(position)
        .filter(|row| row.entry.id == entry.id)
        .ok_or_else(|| error("The captured tree entry index is inconsistent."))?;
    let mut parent = row.parent;
    let mut components = Vec::new();
    let mut base = capture.directory.as_str();
    while let Some(id) = parent {
        check(cancel)?;
        let parent_position = *view
            .index
            .row_positions
            .get(&id)
            .filter(|&&parent_position| parent_position < position)
            .ok_or_else(|| error("The captured tree ancestry is inconsistent."))?;
        let row = view
            .rows
            .get(parent_position)
            .filter(|row| row.id == id)
            .ok_or_else(|| error("The captured tree ancestor is missing."))?;
        if let Some(path) = row.entry.native_path() {
            // An archive expanded inside a native tree starts at its actual
            // container path, rather than the native panel's parent directory.
            base = path.to_str().ok_or_else(|| {
                error("An ancestor path cannot be represented losslessly in Unicode text.")
            })?;
            break;
        }
        components.push(row.entry.name.to_str().ok_or_else(|| {
            error("An ancestor name cannot be represented losslessly in Unicode text.")
        })?);
        parent = row.parent;
        position = parent_position;
    }
    let mut directory = base.to_owned();
    for component in components.into_iter().rev() {
        if !directory.ends_with(std::path::MAIN_SEPARATOR) {
            directory.push(std::path::MAIN_SEPARATOR);
        }
        directory.push_str(component);
    }
    record(entry, &directory, environment)
}

#[cfg(test)]
#[path = "file_list_records_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "file_list_tree_records_tests.rs"]
mod tree_tests;
