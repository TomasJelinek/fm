//! What a Copy/Move draft holds, as the first line of its dialog names it: "Copy
//! “report.pdf” (1.2 MiB) to:", "Move 3 files (12.4 MiB) to:", "Copy 2 files and 1 folder
//! to:". The counts and sizes are those the draft captured with its selection; nothing is
//! read to tell them.
use super::*;

/// What a Copy/Move draft holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DraftItems {
    /// One item, by its name, with its size when it is a file whose size is known.
    One { name: String, bytes: Option<u64> },
    /// Several items: the files (links and other entries among them, as a selection
    /// counts them) and the folders, with the files' total size when every one is known.
    Several {
        files: u64,
        folders: u64,
        bytes: Option<u64>,
    },
    /// Paths of a pasted Cut, whose kinds only the job learns: their number, and the name
    /// of the only one.
    Clipboard { count: u64, name: Option<String> },
}

impl DraftItems {
    /// The marked items of a captured native selection, or the focused item where none
    /// is marked.
    pub(crate) fn selected(files: &SelectedFiles) -> Self {
        if !files.selection.is_empty() {
            return Self::counted(
                files.selection.file_count(),
                files.selection.folder_count(),
                files.selection.file_bytes(),
            );
        }
        let Some(path) = files.fallback.as_deref() else {
            return Self::counted(0, 0, None);
        };
        let bytes = files
            .view
            .by_path
            .get(path)
            .and_then(|id| files.view.by_id.get(id))
            .and_then(|row| files.view.file_sizes.get(*row).copied().flatten());
        Self::One {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            bytes,
        }
    }

    /// Several items, as a selection counts them.
    pub(crate) fn counted(files: usize, folders: usize, bytes: Option<u64>) -> Self {
        Self::Several {
            files: files as u64,
            folders: folders as u64,
            bytes,
        }
    }

    /// One listed entry, with its size when it is a file.
    pub(crate) fn entry(entry: &Entry) -> Self {
        Self::One {
            name: entry.display_name.clone(),
            bytes: (entry.kind == EntryKind::File)
                .then_some(entry.metadata.size)
                .flatten(),
        }
    }

    /// The paths of a pasted Cut.
    pub(crate) fn clipboard(paths: &[PathBuf]) -> Self {
        Self::Clipboard {
            count: paths.len() as u64,
            name: match paths {
                [path] => path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                _ => None,
            },
        }
    }

    /// The dialog's first line: what `intent` does with the items, ending where the
    /// destination field takes over. A name is quoted, shown with its direction controls
    /// escaped and isolated, so that a right-to-left name cannot reorder the line. The
    /// size counts files only, so it is left out where folders are counted.
    pub(crate) fn summary(&self, intent: TransferIntent, locale: &NumberLocale) -> String {
        let verb = match intent {
            TransferIntent::Move => "Move",
            TransferIntent::Copy => "Copy",
        };
        let quoted = |name: &str| {
            format!(
                "\u{201c}{}\u{201d}",
                crate::isolated_name(&crate::shown_text(name))
            )
        };
        let size = |bytes: &Option<u64>| {
            bytes.map_or_else(String::new, |bytes| {
                format!(" ({})", crate::format_size(bytes, locale))
            })
        };
        let count = crate::job_presentation::counted;
        let items = match self {
            Self::One { name, bytes } => format!("{}{}", quoted(name), size(bytes)),
            Self::Several {
                files,
                folders: 0,
                bytes,
            } => format!("{}{}", count(*files, "file", "files", locale), size(bytes)),
            Self::Several {
                files: 0, folders, ..
            } => count(*folders, "folder", "folders", locale),
            Self::Several { files, folders, .. } => format!(
                "{} and {}",
                count(*files, "file", "files", locale),
                count(*folders, "folder", "folders", locale)
            ),
            Self::Clipboard {
                name: Some(name), ..
            } => format!("{} from the clipboard", quoted(name)),
            Self::Clipboard { count: items, .. } => format!(
                "{} from the clipboard",
                count(*items, "item", "items", locale)
            ),
        };
        format!("{verb} {items} to:")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(items: DraftItems, intent: TransferIntent) -> String {
        let locale = NumberLocale {
            group_separator: ",".into(),
            grouping: vec![3],
            repeat_last_group: true,
            ..NumberLocale::default()
        };
        items.summary(intent, &locale)
    }

    #[test]
    fn one_item_is_named_with_the_size_of_a_file() {
        assert_eq!(
            line(
                DraftItems::One {
                    name: "report.pdf".into(),
                    bytes: Some(1_234_567)
                },
                TransferIntent::Copy
            ),
            "Copy \u{201c}\u{2066}report.pdf\u{2069}\u{201d} (1.2 MiB) to:"
        );
        assert_eq!(
            line(
                DraftItems::One {
                    name: "Photos".into(),
                    bytes: None
                },
                TransferIntent::Move
            ),
            "Move \u{201c}\u{2066}Photos\u{2069}\u{201d} to:"
        );
        // A name's direction controls are shown escaped, inside the isolate.
        let spoof = line(
            DraftItems::One {
                name: "photo\u{202e}gpj.exe".into(),
                bytes: None,
            },
            TransferIntent::Copy,
        );
        assert!(!spoof.contains('\u{202e}'), "{spoof}");
        assert!(spoof.contains("photo\\u202Egpj.exe"), "{spoof}");
    }

    #[test]
    fn several_items_are_counted_by_kind_and_sized_only_without_folders() {
        for (files, folders, bytes, expected) in [
            (3, 0, Some(13_002_342), "Copy 3 files (12.4 MiB) to:"),
            (1, 0, Some(1), "Copy 1 file (1 B) to:"),
            (2_000, 0, None, "Copy 2,000 files to:"),
            (0, 2, None, "Copy 2 folders to:"),
            (0, 1, None, "Copy 1 folder to:"),
            (2, 1, Some(10), "Copy 2 files and 1 folder to:"),
            (1, 3, None, "Copy 1 file and 3 folders to:"),
        ] {
            assert_eq!(
                line(
                    DraftItems::Several {
                        files,
                        folders,
                        bytes
                    },
                    TransferIntent::Copy
                ),
                expected
            );
        }
    }

    #[test]
    fn clipboard_paths_say_where_they_come_from() {
        assert_eq!(
            line(
                DraftItems::clipboard(&[PathBuf::from("a"), PathBuf::from("b")]),
                TransferIntent::Move
            ),
            "Move 2 items from the clipboard to:"
        );
        assert_eq!(
            line(
                DraftItems::clipboard(&[PathBuf::from("dir").join("report.pdf")]),
                TransferIntent::Move
            ),
            "Move \u{201c}\u{2066}report.pdf\u{2069}\u{201d} from the clipboard to:"
        );
    }
}
