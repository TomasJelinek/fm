//! Settings whose paths belong to another operating system. Hot paths and the
//! default editor are kept as their stored records; tabs at such folders are not
//! opened. Nothing here converts a foreign path into a native path.
use super::{
    NativePath, StoredArchiveAddress, StoredBookmark, StoredEditor, StoredPaneTabs, invalid,
};
use midnight_core::{
    BookmarkId, EditorPreferences, ForeignHotPath, ForeignSetting, ForeignSettingKind,
    MAX_BOOKMARK_NAME_BYTES,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

/// Stored form of a kept default editor: its program and arguments.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEditorProgram {
    executable: NativePath,
    arguments: Vec<String>,
}

/// Kept list entries in stored order, for reinsertion at their positions.
pub(super) fn positioned(
    foreign: &[ForeignSetting],
    position: impl Fn(&ForeignSettingKind) -> Option<usize>,
) -> Vec<(usize, &str)> {
    let mut entries: Vec<_> = foreign
        .iter()
        .filter_map(|setting| position(&setting.kind).map(|at| (at, &*setting.stored)))
        .collect();
    entries.sort_by_key(|(at, _)| *at);
    entries
}

fn container_location(archive: &StoredArchiveAddress, container: &str) -> String {
    let mut location = format!("{container}@");
    for (index, component) in archive.components.iter().enumerate() {
        if index > 0 {
            location.push('/');
        }
        location.push_str(&String::from_utf8_lossy(component));
    }
    location
}

/// A hot path whose folder or archive belongs to another system, validated except
/// for that location and kept as its stored record.
pub(super) fn hot_path(bookmark: &StoredBookmark) -> io::Result<Option<ForeignHotPath>> {
    let container = match &bookmark.archive {
        Some(archive) => archive.container.foreign()?,
        None => None,
    };
    let Some((system, path)) = bookmark.path.foreign()?.or_else(|| container.clone()) else {
        return Ok(None);
    };
    if bookmark.id == 0
        || bookmark.name.trim().is_empty()
        || bookmark.name.len() > MAX_BOOKMARK_NAME_BYTES
        || bookmark.name.chars().any(char::is_control)
        || bookmark.slot.is_some_and(|slot| !(1..=10).contains(&slot))
        || (bookmark.archive.is_some() && bookmark.network.is_some())
    {
        return Err(invalid("invalid hot path from another system"));
    }
    let location = if let Some(network) = &bookmark.network {
        network.clone().into_address()?.display
    } else if let Some(archive) = &bookmark.archive {
        // Validate the address with a native stand-in container; then discard it.
        let mut check = archive.clone();
        check.container = check.container.validation_copy()?;
        check.into_address()?;
        let container = container.map_or(path, |(_, text)| text);
        container_location(archive, &container)
    } else {
        path
    };
    Ok(Some(ForeignHotPath {
        id: BookmarkId(bookmark.id),
        slot: bookmark.slot,
        system,
        location,
        stored: serde_json::to_string(bookmark).map_err(invalid)?.into(),
    }))
}

/// Kept hot paths as stored records, with their current identities and slots.
pub(super) fn stored_hot_paths(paths: &[ForeignHotPath]) -> io::Result<Vec<StoredBookmark>> {
    paths
        .iter()
        .map(|path| {
            let mut bookmark: StoredBookmark =
                serde_json::from_str(&path.stored).map_err(invalid)?;
            bookmark.id = path.id.0;
            bookmark.slot = path.slot;
            Ok(bookmark)
        })
        .collect()
}

/// Tabs at folders of another system are not opened: they are left out, and the
/// nearest remaining tab before the active one becomes active. Returns the count.
pub(super) fn split_tabs(tabs: StoredPaneTabs) -> io::Result<(StoredPaneTabs, usize)> {
    let count = tabs.paths.len();
    if (count == 0 && tabs.active != 0)
        || (count > 0 && tabs.active >= count)
        || tabs
            .archives
            .keys()
            .chain(tabs.networks.keys())
            .any(|index| *index >= count)
    {
        return Err(invalid(
            "tab addresses and the active tab must refer to saved tabs",
        ));
    }
    let mut kept = Vec::with_capacity(count);
    for (index, path) in tabs.paths.iter().enumerate() {
        let container = match tabs.archives.get(&index) {
            Some(archive) => archive.container.foreign()?,
            None => None,
        };
        kept.push(path.foreign()?.is_none() && container.is_none());
    }
    let skipped = kept.iter().filter(|kept| !**kept).count();
    if skipped == 0 {
        return Ok((tabs, 0));
    }
    let mut renumbered = BTreeMap::new();
    for (index, _) in kept.iter().enumerate().filter(|(_, kept)| **kept) {
        renumbered.insert(index, renumbered.len());
    }
    let active = renumbered
        .range(..=tabs.active)
        .next_back()
        .or_else(|| renumbered.iter().next())
        .map_or(0, |(_, index)| *index);
    Ok((
        StoredPaneTabs {
            paths: tabs
                .paths
                .into_iter()
                .zip(&kept)
                .filter_map(|(path, kept)| kept.then_some(path))
                .collect(),
            active,
            archives: renumber(tabs.archives, &renumbered),
            networks: renumber(tabs.networks, &renumbered),
        },
        skipped,
    ))
}

fn renumber<T>(
    entries: BTreeMap<usize, T>,
    renumbered: &BTreeMap<usize, usize>,
) -> BTreeMap<usize, T> {
    entries
        .into_iter()
        .filter_map(|(index, value)| renumbered.get(&index).map(|new| (*new, value)))
        .collect()
}

/// Take a default editor of another system out of `editor`, which then uses the
/// system editor; its arguments are validated with a native stand-in program.
pub(super) fn take_editor_program(editor: &mut StoredEditor) -> io::Result<Option<ForeignSetting>> {
    let foreign = match &editor.executable {
        Some(executable) => executable.foreign()?,
        None => None,
    };
    let Some((system, location)) = foreign else {
        return Ok(None);
    };
    let Some(executable) = editor.executable.take() else {
        return Ok(None);
    };
    let arguments = std::mem::replace(
        &mut editor.arguments,
        EditorPreferences::default().arguments,
    );
    EditorPreferences {
        executable: Some(NativePath::stand_in().into_path()?),
        arguments: arguments.clone(),
        rules: Vec::new(),
    }
    .validate()
    .map_err(invalid)?;
    let program = StoredEditorProgram {
        executable,
        arguments,
    };
    Ok(Some(ForeignSetting {
        kind: ForeignSettingKind::Editor,
        system,
        location,
        stored: serde_json::to_string(&program).map_err(invalid)?.into(),
    }))
}

/// The kept default editor, written back only while no native editor replaced it.
pub(super) fn editor_program(
    native: bool,
    foreign: &[ForeignSetting],
) -> io::Result<Option<(NativePath, Vec<String>)>> {
    if native {
        return Ok(None);
    }
    foreign
        .iter()
        .find(|setting| setting.kind == ForeignSettingKind::Editor)
        .map(|setting| {
            let program: StoredEditorProgram =
                serde_json::from_str(&setting.stored).map_err(invalid)?;
            Ok((program.executable, program.arguments))
        })
        .transpose()
}

#[cfg(test)]
#[path = "foreign_tests.rs"]
mod tests;
