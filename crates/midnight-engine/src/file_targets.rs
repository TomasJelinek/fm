//! Bounded expansion of a captured selection. Never call on the UI thread.
use midnight_core::{
    Cancellation, DirectoryIndex, FsError, MAX_FILE_CLIPBOARD_BYTES, SelectionSnapshot,
};
use std::path::{Component, Path, PathBuf};

/// Items that a menu, dialog or Custom Command takes from one panel selection: each
/// holds something per item (a shell item, a dialog entry, an argument). Checksums take
/// any number (`resolve_all`).
pub(crate) const MAX_ITEMS: usize = 4096;
// The refusal in `resolve_within` names this bound.
const _: () = assert!(MAX_ITEMS == 4096);

/// The selected paths in display order, at most `MAX_ITEMS` of them, for consumers that
/// hold something per item. Native paths take at most `MAX_FILE_CLIPBOARD_BYTES`.
pub(crate) fn resolve(
    selection: &SelectionSnapshot,
    view: &DirectoryIndex,
    fallback: Option<&Path>,
    cancel: &Cancellation,
) -> Result<Vec<PathBuf>, FsError> {
    resolve_within(
        selection,
        view,
        fallback,
        cancel,
        Some(MAX_ITEMS),
        Some(MAX_FILE_CLIPBOARD_BYTES),
    )
}

/// The selected paths in display order, any number of them whatever their names' total
/// length: for checksums, whose runs have no size limit and keep compact rows per file.
pub(crate) fn resolve_all(
    selection: &SelectionSnapshot,
    view: &DirectoryIndex,
    fallback: Option<&Path>,
    cancel: &Cancellation,
) -> Result<Vec<PathBuf>, FsError> {
    resolve_within(selection, view, fallback, cancel, None, None)
}

/// The selected paths in display order for a native file list (the clipboard, a drag
/// source, a Copy or Move that bounds its roots itself): any number of them, within the
/// list's `MAX_FILE_CLIPBOARD_BYTES` of names, which also bound this copy of them.
pub(crate) fn resolve_payload(
    selection: &SelectionSnapshot,
    view: &DirectoryIndex,
    fallback: Option<&Path>,
    cancel: &Cancellation,
) -> Result<Vec<PathBuf>, FsError> {
    resolve_within(
        selection,
        view,
        fallback,
        cancel,
        None,
        Some(MAX_FILE_CLIPBOARD_BYTES),
    )
}

fn resolve_within(
    selection: &SelectionSnapshot,
    view: &DirectoryIndex,
    fallback: Option<&Path>,
    cancel: &Cancellation,
    items: Option<usize>,
    name_bytes: Option<usize>,
) -> Result<Vec<PathBuf>, FsError> {
    use std::io;
    let failure = |kind, message| {
        FsError::from_io(
            "file selection",
            Path::new(""),
            io::Error::new(kind, message),
        )
    };
    let mut paths = Vec::new();
    let mut bytes = 0_usize;
    let mut add = |position, path: &Path| {
        if cancel.is_cancelled() {
            return Err(failure(
                io::ErrorKind::Interrupted,
                "File selection cancelled.",
            ));
        }
        if path.as_os_str().is_empty()
            || path.components().next_back() == Some(Component::ParentDir)
        {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "An empty path or parent navigation is not a file-operation target.",
            ));
        }
        let raw = path.as_os_str().as_encoded_bytes();
        bytes = bytes.saturating_add(raw.len());
        if items.is_some_and(|items| paths.len() >= items) {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "This action takes at most 4096 selected items at once.",
            ));
        }
        if name_bytes.is_some_and(|limit| bytes > limit) || raw.contains(&0) {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "The selected names exceed what a file list holds, or one contains NUL.",
            ));
        }
        paths.push((position, path.to_owned()));
        Ok(())
    };
    if selection.is_empty() {
        if let Some(path) = fallback {
            add(0, path)?;
        }
    } else {
        for (path, id) in &view.by_path {
            if cancel.is_cancelled() {
                return Err(failure(
                    io::ErrorKind::Interrupted,
                    "File selection cancelled.",
                ));
            }
            if selection.contains_path(path) {
                let position = view.by_id.get(id).ok_or_else(|| {
                    failure(
                        io::ErrorKind::InvalidData,
                        "Captured view has an invalid entry identity.",
                    )
                })?;
                add(*position, path)?;
            }
        }
    }
    if paths.is_empty() {
        return Err(failure(
            // This is validation of a frozen selection, not a transient missing
            // filesystem object. Repeating it cannot discover different targets.
            io::ErrorKind::InvalidInput,
            "No selected entries remain in the captured folder.",
        ));
    }
    paths.sort_unstable_by_key(|(position, _)| *position);
    Ok(paths.into_iter().map(|(_, path)| path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{Entry, EntryId, EntryKind};
    use std::sync::Arc;

    fn index(names: &[&str]) -> Arc<DirectoryIndex> {
        Arc::new(DirectoryIndex::from_entries(
            &names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    Entry::new(EntryId(i as u64), PathBuf::from(name), EntryKind::File)
                })
                .collect::<Vec<_>>(),
        ))
    }

    #[test]
    fn captured_view_order_and_literal_paths_survive_filtering_and_reused_ids() {
        let original = index(&["z", "文档 $(literal)", "removed", "a"]);
        let selected = SelectionSnapshot::from_bits(original, vec![15]).unwrap();
        let view = index(&["a", "new item", "文档 $(literal)", "z"]);
        assert_eq!(
            resolve(
                &selected,
                &view,
                Some(Path::new("ignored")),
                &Cancellation::default()
            )
            .unwrap(),
            vec![PathBuf::from("a"), "文档 $(literal)".into(), "z".into()]
        );
    }

    #[test]
    fn parent_navigation_paths_are_rejected_before_native_file_actions() {
        for name in ["", "..", "folder/.."] {
            let view = index(&[name]);
            let selected = SelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap();
            assert!(resolve(&selected, &view, None, &Cancellation::default()).is_err());
            assert!(
                resolve(
                    &SelectionSnapshot::default(),
                    &view,
                    Some(Path::new(name)),
                    &Cancellation::default()
                )
                .is_err()
            );
        }
        // A real listed entry can retain an unnormalized directory prefix,
        // for example when the application was opened with --left ../other.
        let path = Path::new("folder/../other/file.txt");
        assert_eq!(
            resolve(
                &SelectionSnapshot::default(),
                &index(&[]),
                Some(path),
                &Cancellation::default()
            )
            .unwrap(),
            vec![path.to_path_buf()]
        );
    }

    #[test]
    fn fallback_only_applies_to_an_empty_selection_and_limits_reject_whole_selection() {
        let view = index(&["focus"]);
        assert_eq!(
            resolve(
                &SelectionSnapshot::default(),
                &view,
                Some(Path::new("focus")),
                &Cancellation::default()
            )
            .unwrap(),
            vec![PathBuf::from("focus")]
        );
        let selected = SelectionSnapshot::from_bits(index(&["gone"]), vec![1]).unwrap();
        assert!(
            resolve(
                &selected,
                &view,
                Some(Path::new("focus")),
                &Cancellation::default()
            )
            .is_err()
        );
        let names: Vec<_> = (0..4097).map(|i| format!("file-{i}")).collect();
        let large = index(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let mut bits = vec![u64::MAX; 65];
        bits[64] = 1;
        let selected = SelectionSnapshot::from_bits(large.clone(), bits).unwrap();
        assert!(resolve(&selected, &large, None, &Cancellation::default()).is_err());
        // A native file list takes them all: only its names' bytes bound it.
        let all = resolve_payload(&selected, &large, None, &Cancellation::default()).unwrap();
        assert_eq!(all.len(), 4097);
        assert_eq!(all[4096], PathBuf::from("file-4096"));
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert!(
            resolve(
                &SelectionSnapshot::default(),
                &view,
                Some(Path::new("focus")),
                &cancelled
            )
            .is_err()
        );
    }

    /// Checksums take any selection: more items than 4,096 and more names than the 4 MiB
    /// a file list holds. An empty path, parent navigation or NUL is still refused.
    #[test]
    fn checksums_take_any_number_of_items_and_names_of_any_total_length() {
        // More names than a bound of 4 MiB holds (the file-list bound once).
        let names: Vec<_> = (0..5_000)
            .map(|i| format!("{i:05}-{}", "n".repeat(994)))
            .collect();
        let bound = 4 << 20;
        assert!(names.iter().map(String::len).sum::<usize>() > bound);
        let large = index(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let mut bits = vec![u64::MAX; 79];
        bits[78] = 0xff;
        let selected = SelectionSnapshot::from_bits(large.clone(), bits).unwrap();
        let cancel = Cancellation::default();
        assert!(resolve(&selected, &large, None, &cancel).is_err());
        assert!(resolve_within(&selected, &large, None, &cancel, None, Some(bound)).is_err());
        assert_eq!(
            resolve_payload(&selected, &large, None, &cancel)
                .unwrap()
                .len(),
            5_000
        );
        let all = resolve_all(&selected, &large, None, &cancel).unwrap();
        assert_eq!(all.len(), 5_000);
        assert_eq!(all[4_999], PathBuf::from(&names[4_999]));
        for name in ["", "folder/..", "nul\0name"] {
            let view = index(&[name]);
            let selected = SelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap();
            assert!(
                resolve_all(&selected, &view, None, &cancel).is_err(),
                "{name:?}"
            );
        }
    }
}
