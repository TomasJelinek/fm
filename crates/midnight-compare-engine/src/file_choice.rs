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
#[path = "archive_input.rs"]
pub(crate) mod archive_input;
use midnight_core::{ArchiveCompareFolder, ArchiveCompareMember, DirectoryIndex, Entry, EntryId};

#[derive(Clone, Debug)]
pub(crate) enum FileInput {
    Native(PathBuf),
    Archive(ArchiveCompareMember),
}

impl FileInput {
    /// Policy matches complete names. Decoder discovery needs only an extension, so
    /// an opaque archive stem can still select a decoder for a known ASCII suffix.
    pub(crate) fn routing_name(&self, decoder_only: bool) -> PathBuf {
        let Self::Archive(member) = self else {
            return self.draft();
        };
        let raw = member.node.raw_name.as_ref();
        if raw.iter().any(|byte| b"/\\:\0".contains(byte)) {
            return PathBuf::new();
        }
        if let Ok(name) = std::str::from_utf8(raw) {
            return name.into();
        }
        if decoder_only
            && let Some(dot) = raw.iter().rposition(|&byte| byte == b'.')
            && dot > 0
            && let Ok(extension) = std::str::from_utf8(&raw[dot + 1..])
        {
            return format!("member.{extension}").into();
        }
        PathBuf::new()
    }

    /// The chooser edits native paths. An unchanged archive label on the same side
    /// retains the original member capability, including when labels are duplicates.
    pub(crate) fn draft(&self) -> PathBuf {
        match self {
            Self::Native(path) => path.clone(),
            Self::Archive(member) => {
                format!("Archive member · {}", member.node.display_name).into()
            }
        }
    }

    fn raw_name(&self) -> Result<&[u8], String> {
        match self {
            Self::Native(path) => path
                .file_name()
                .map(|name| name.as_encoded_bytes())
                .ok_or_else(|| "The active file has no file name.".into()),
            Self::Archive(member) => Ok(&member.node.raw_name),
        }
    }
}

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
            external_outcome: None,
            message: result.err().unwrap_or_default(),
        }
    }
}

pub(crate) fn validate(inputs: &[FileInput; 2]) -> Result<(), String> {
    for (side, input) in inputs.iter().enumerate() {
        let FileInput::Native(path) = input else {
            continue;
        };
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

#[cfg(test)]
pub(crate) fn resolve(
    launch: &FileCompareLaunch,
    cancel: &AtomicBool,
) -> Result<[PathBuf; 2], FileChoice> {
    let (inputs, message) = resolve_inputs(launch, cancel);
    let paths = inputs.each_ref().map(FileInput::draft);
    match message {
        None => Ok(paths),
        Some(message) => Err(FileChoice {
            paths,
            message,
            is_error: false,
            external_outcome: None,
        }),
    }
}

pub(crate) fn resolve_inputs(
    launch: &FileCompareLaunch,
    cancel: &AtomicBool,
) -> ([FileInput; 2], Option<String>) {
    let mut sides: [Vec<FileInput>; 2] = Default::default();
    let mut error = None;
    if launch.sources.len() > 2 {
        return (
            std::array::from_fn(|_| FileInput::Native(PathBuf::new())),
            Some("Choose two files to compare.".into()),
        );
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
        if launch.sources.len() != 2 || sides[1 - side].len() != 1 {
            continue;
        }
        let other = &sides[1 - side][0];
        let matched = match source {
            FileCompareSource::SameName {
                directory,
                entries,
                view,
            } => same_name(directory, entries, view, None, other, cancel).map(FileInput::Native),
            FileCompareSource::RootSameName {
                directory,
                entries,
                view,
                visible,
            } => same_name(directory, entries, view, Some(visible), other, cancel)
                .map(FileInput::Native),
            FileCompareSource::ArchiveSameName {
                folder,
                entries,
                view,
                visible,
            } => archive_same_name(folder, entries, view, visible.as_deref(), other, cancel)
                .map(FileInput::Archive),
            _ => continue,
        };
        match matched {
            Ok(path) => sides[side].push(path),
            Err(message) => {
                error.get_or_insert(message);
            }
        }
    }
    // Two selected files in one panel retain their visible order. Across panels,
    // keep the known file in its physical A/B slot even when the other is missing.
    let inputs = if launch.sources.len() == 1 && sides[0].len() == 2 {
        [sides[0].remove(0), sides[0].remove(0)]
    } else {
        sides.map(|mut paths| {
            if paths.len() == 1 {
                paths.remove(0)
            } else {
                FileInput::Native(PathBuf::new())
            }
        })
    };
    if inputs
        .iter()
        .any(|input| matches!(input, FileInput::Native(path) if path.as_os_str().is_empty()))
    {
        error.get_or_insert_with(|| "Choose file A and file B to compare.".into());
    }
    (inputs, error)
}

fn resolve_side(source: &FileCompareSource, cancel: &AtomicBool) -> Result<Vec<FileInput>, String> {
    check_cancel(cancel)?;
    match source {
        FileCompareSource::Unspecified
        | FileCompareSource::ArchiveSameName { .. }
        | FileCompareSource::SameName { .. }
        | FileCompareSource::RootSameName { .. } => Ok(Vec::new()),
        FileCompareSource::Focused(path) => Ok(vec![FileInput::Native(path.clone())]),
        FileCompareSource::ArchiveMember(member) => {
            archive_input::check(member)?;
            Ok(vec![FileInput::Archive(member.clone())])
        }
        FileCompareSource::ArchiveSelected {
            folder,
            entries,
            index,
            view,
            selection,
        } => {
            if selection.len() > 2 {
                return Err("The panel selection is ambiguous. Choose two files below.".into());
            }
            let mut selected = Vec::with_capacity(selection.len());
            for id in selection.iter() {
                check_cancel(cancel)?;
                let Some((entry, &row)) = index
                    .by_id
                    .get(id)
                    .and_then(|&i| entries.get(i))
                    .zip(view.by_id.get(id))
                else {
                    return Err("The selected member is no longer visible.".into());
                };
                let node = entry
                    .virtual_node
                    .as_deref()
                    .filter(|node| entry.id == *id && node.id.entry == id.0)
                    .ok_or("The selected archive member identity is unavailable.")?;
                let member = ArchiveCompareMember {
                    folder: folder.clone(),
                    node: node.clone(),
                };
                archive_input::check(&member)?;
                selected.push((row, FileInput::Archive(member)));
            }
            selected.sort_unstable_by_key(|(row, _)| *row);
            Ok(selected.into_iter().map(|(_, input)| input).collect())
        }
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
                    if entry.kind != EntryKind::File || entry.native_path().is_none() {
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
            Ok(selected
                .into_iter()
                .map(|(_, path)| FileInput::Native(path))
                .collect())
        }
    }
}

fn same_name(
    directory: &std::path::Path,
    entries: &[midnight_core::Entry],
    view: &midnight_core::DirectoryIndex,
    visible: Option<&[midnight_core::EntryId]>,
    source: &FileInput,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let name = source.raw_name()?;
    let mut found = None;
    for entry in entries {
        check_cancel(cancel)?;
        if entry.name.as_encoded_bytes() != name
            || !view.by_id.contains_key(&entry.id)
            || visible.is_some_and(|ids| ids.binary_search_by_key(&entry.id.0, |id| id.0).is_err())
        {
            continue;
        }
        if entry.kind != EntryKind::File || entry.native_path().is_none() {
            return Err(
                "The same-name entry is not an ordinary native file. Choose another file below."
                    .into(),
            );
        }
        if entry.path.parent() != Some(directory)
            || entry.path.file_name() != Some(entry.name.as_os_str())
        {
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

fn archive_same_name(
    folder: &ArchiveCompareFolder,
    entries: &[Entry],
    view: &DirectoryIndex,
    visible: Option<&[EntryId]>,
    other: &FileInput,
    cancel: &AtomicBool,
) -> Result<ArchiveCompareMember, String> {
    let name = other.raw_name()?;
    let mut found = None;
    for entry in entries {
        check_cancel(cancel)?;
        let Some(node) = entry.virtual_node.as_deref() else {
            continue;
        };
        if node.raw_name.as_ref() != name
            || !view.by_id.contains_key(&entry.id)
            || visible.is_some_and(|ids| ids.binary_search_by_key(&entry.id.0, |id| id.0).is_err())
        {
            continue;
        }
        if entry.id.0 != node.id.entry || entry.kind != node.kind {
            return Err("The captured archive view has inconsistent member identities.".into());
        }
        let member = ArchiveCompareMember {
            folder: folder.clone(),
            node: node.clone(),
        };
        archive_input::check(&member)?;
        if found.replace(member).is_some() {
            return Err(
                "The same-name comparison is ambiguous. Choose the second file below.".into(),
            );
        }
    }
    found.ok_or_else(|| {
        "No same-name file is visible in the opposite panel. Choose the second file below.".into()
    })
}
