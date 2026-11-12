//! Pair resolution and native picking stay on the bounded comparator worker.
use crate::{FileChoice, check_cancel};
use midnight_core::{
    Cancellation, EntryKind, FileCompareLaunch, FileCompareSource, PathPickerKind, PathPickerOwner,
    PathPickerRequest, PathPickerService,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

#[cfg(test)]
#[path = "file_choice_tests.rs"]
mod tests;

pub(crate) struct Picker {
    pub service: Option<Arc<dyn PathPickerService>>,
    pub cancel: Cancellation,
}

impl Picker {
    pub fn browse(
        &self,
        side: usize,
        mut paths: [PathBuf; 2],
        owner: PathPickerOwner,
    ) -> FileChoice {
        let result = (|| {
            let path = paths.get(side).ok_or("Unknown comparison side.")?;
            let service = self
                .service
                .as_ref()
                .ok_or("The system file picker is unavailable. Enter an absolute path instead.")?;
            let request = PathPickerRequest {
                kind: PathPickerKind::File,
                title: format!(
                    "Choose file {} to compare",
                    if side == 0 { "A" } else { "B" }
                ),
                initial_directory: path
                    .parent()
                    .filter(|path| path.is_absolute())
                    .map(ToOwned::to_owned),
                owner,
            };
            request.validate().map_err(|error| error.to_string())?;
            if let Some(path) = service
                .pick_path(&request, &self.cancel)
                .map_err(|error| error.to_string())?
            {
                midnight_core::validate_picked_path(&path).map_err(|error| error.to_string())?;
                paths[side] = path;
            }
            Ok::<_, String>(())
        })();
        FileChoice {
            paths,
            is_error: result.is_err(),
            message: result.err().unwrap_or_default(),
        }
    }
}

pub(crate) fn validate(paths: &[PathBuf; 2]) -> Result<(), String> {
    for (side, path) in paths.iter().enumerate() {
        midnight_core::validate_picked_path(path).map_err(|error| {
            format!(
                "File {}: {}\n{}",
                if side == 0 { "A" } else { "B" },
                error.message,
                path.display()
            )
        })?;
    }
    Ok(())
}

pub(crate) fn resolve(
    launch: &FileCompareLaunch,
    cancel: &AtomicBool,
) -> Result<[PathBuf; 2], FileChoice> {
    let mut sides: [Vec<PathBuf>; 2] = Default::default();
    let mut error = None;
    if launch.sources.len() > 2 {
        return Err(FileChoice {
            paths: Default::default(),
            message: "Choose two files to compare.".into(),
            is_error: false,
        });
    }
    for (side, source) in launch.sources.iter().enumerate() {
        match resolve_side(source, cancel) {
            Ok(paths) => sides[side] = paths,
            Err(message) => {
                error.get_or_insert(message);
            }
        }
    }
    for (side, source) in launch.sources.iter().enumerate() {
        let (directory, entries, view, visible) = match source {
            FileCompareSource::SameName {
                directory,
                entries,
                view,
            } => (directory, entries, view, None),
            FileCompareSource::RootSameName {
                directory,
                entries,
                view,
                visible,
            } => (directory, entries, view, Some(visible.as_ref())),
            _ => continue,
        };
        if launch.sources.len() == 2 && sides[1 - side].len() == 1 {
            match same_name(
                directory,
                entries,
                view,
                visible,
                &sides[1 - side][0],
                cancel,
            ) {
                Ok(path) => sides[side].push(path),
                Err(message) => {
                    error.get_or_insert(message);
                }
            }
        }
    }
    // Two selected files in one panel retain their visible order. Across panels,
    // keep the known file in its physical A/B slot even when the other is missing.
    let paths: [PathBuf; 2] = if launch.sources.len() == 1 && sides[0].len() == 2 {
        [sides[0].remove(0), sides[0].remove(0)]
    } else {
        sides.map(|mut paths| {
            if paths.len() == 1 {
                paths.remove(0)
            } else {
                PathBuf::new()
            }
        })
    };
    if error.is_none() && paths.iter().all(|path| !path.as_os_str().is_empty()) {
        Ok(paths)
    } else {
        Err(FileChoice {
            paths,
            is_error: false,
            message: error.unwrap_or_else(|| "Choose file A and file B to compare.".into()),
        })
    }
}

fn resolve_side(source: &FileCompareSource, cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
    check_cancel(cancel)?;
    match source {
        FileCompareSource::Unspecified
        | FileCompareSource::SameName { .. }
        | FileCompareSource::RootSameName { .. } => Ok(Vec::new()),
        FileCompareSource::Focused(path) => Ok(vec![path.clone()]),
        FileCompareSource::Selected {
            entries,
            view,
            selection,
        } => {
            let mut selected = Vec::with_capacity(2);
            for entry in entries.iter() {
                check_cancel(cancel)?;
                if selection.contains_path(&entry.path)
                    && let Some(row) = view.by_id.get(&entry.id)
                {
                    if entry.kind != EntryKind::File || entry.virtual_node.is_some() {
                        return Err(
                            "Comparison requires ordinary native files. Choose files below.".into(),
                        );
                    }
                    selected.push((*row, entry.path.clone()));
                    if selected.len() > 2 {
                        return Err(
                            "The panel selection is ambiguous. Choose two files below.".into()
                        );
                    }
                }
            }
            if selected.len() != selection.len() {
                return Err("The selected files are no longer present in the captured view. Choose files below.".into());
            }
            selected.sort_unstable_by_key(|(row, _)| *row);
            Ok(selected.into_iter().map(|(_, path)| path).collect())
        }
    }
}

fn same_name(
    directory: &std::path::Path,
    entries: &[midnight_core::Entry],
    view: &midnight_core::DirectoryIndex,
    visible: Option<&[midnight_core::EntryId]>,
    source: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let name = source
        .file_name()
        .ok_or("The active file has no native file name.")?;
    let mut found = None;
    for entry in entries {
        check_cancel(cancel)?;
        if entry.name != name
            || !view.by_id.contains_key(&entry.id)
            || visible.is_some_and(|ids| ids.binary_search_by_key(&entry.id.0, |id| id.0).is_err())
        {
            continue;
        }
        if entry.kind != EntryKind::File || entry.virtual_node.is_some() {
            return Err(
                "The same-name entry is not an ordinary native file. Choose another file below."
                    .into(),
            );
        }
        if entry.path.parent() != Some(directory) || entry.path.file_name() != Some(name) {
            return Err("The same-name entry does not belong to the captured folder.".into());
        }
        if found.replace(entry).is_some() {
            return Err(
                "The same-name comparison is ambiguous. Choose the second file below.".into(),
            );
        }
    }
    let entry = found.ok_or(
        "No same-name file is visible in the opposite panel. Choose the second file below.",
    )?;
    if view.by_path.get(&entry.path) != Some(&entry.id) {
        return Err("The captured comparison view has inconsistent file identities.".into());
    }
    Ok(entry.path.clone())
}
