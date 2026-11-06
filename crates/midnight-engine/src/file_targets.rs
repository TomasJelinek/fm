//! Bounded expansion of a captured selection. Never call on the UI thread.
use midnight_core::{Cancellation, DirectoryIndex, FsError, SelectionSnapshot};
use std::path::{Path, PathBuf};

pub(crate) fn resolve(
    selection: &SelectionSnapshot,
    view: &DirectoryIndex,
    fallback: Option<&Path>,
    cancel: &Cancellation,
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
        let raw = path.as_os_str().as_encoded_bytes();
        bytes = bytes.saturating_add(raw.len());
        if paths.len() >= 4096 || bytes > 4 * 1024 * 1024 || raw.contains(&0) {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "File selection exceeds its clipboard/menu resource budget or contains NUL.",
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
            io::ErrorKind::NotFound,
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
}
