//! Constant-size delete presentation; the worker owns and summarizes the full target list.
use midnight_core::{DeleteMode, EntryKind, MutationItem, NumberLocale, OperationPreview};

/// The single target, when the confirmation deletes exactly one item.
fn single(preview: &OperationPreview) -> Option<&MutationItem> {
    (preview.files.checked_add(preview.folders) == Some(1))
        .then(|| preview.sample.first())
        .flatten()
}

fn name(item: &MutationItem) -> std::borrow::Cow<'_, str> {
    item.path
        .file_name()
        .unwrap_or(item.path.as_os_str())
        .to_string_lossy()
}

/// What one target is, as the summary and the question name it. The worker counts
/// directory links with folders and other links with files; both are links here.
fn noun(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "folder",
        EntryKind::File => "file",
        EntryKind::DirectoryLink | EntryKind::Symlink => "link",
        EntryKind::Other => "item",
    }
}

pub(super) fn summary(preview: &OperationPreview, locale: &NumberLocale) -> String {
    if let Some(item) = single(preview) {
        let name = name(item);
        let modified = crate::time_presentation::modified(&item.metadata)
            .map(|value| format!("{} {}", value.date, value.time))
            .unwrap_or_else(|| "Unavailable".into());
        let noun = noun(item.kind);
        return if item.kind == EntryKind::File {
            format!(
                "1 {noun}\nName: {name}\nSize: {}\nModified: {modified}",
                bytes(item.metadata.size, locale)
            )
        } else {
            format!("1 {noun}\nName: {name}\nModified: {modified}")
        };
    }
    let mut text = String::new();
    if preview.files != 0 || preview.folders == 0 {
        text = count(preview.files, "file", locale);
    }
    if preview.folders != 0 {
        if !text.is_empty() {
            text.push_str(" and ");
        }
        text.push_str(&count(preview.folders, "folder", locale));
    }
    if preview.files != 0 {
        text.push_str(&format!(
            "\nFiles size: {}",
            bytes(preview.file_bytes, locale)
        ));
    }
    text
}

/// The confirmation question: one target is named by its kind, several are counted.
/// That a permanent delete cannot be undone is the dialog's warning line, said once.
pub(super) fn question(
    preview: &OperationPreview,
    mode: DeleteMode,
    locale: &NumberLocale,
) -> String {
    if let Some(item) = single(preview) {
        let (noun, name) = (noun(item.kind), name(item));
        return match (mode, item.kind) {
            (DeleteMode::Permanent, EntryKind::Directory) => {
                format!("Permanently delete the folder “{name}” and everything in it?")
            }
            (DeleteMode::Permanent, EntryKind::DirectoryLink | EntryKind::Symlink) => {
                format!("Permanently delete the link “{name}”? What it points to is not deleted.")
            }
            (DeleteMode::Permanent, _) => format!("Permanently delete the {noun} “{name}”?"),
            (DeleteMode::Trash, _) => format!(
                "Move the {noun} “{name}” to the Trash? If the Trash is unavailable, it stays in place."
            ),
        };
    }
    let items = match preview.files.checked_add(preview.folders) {
        Some(1) => "this item".to_owned(),
        Some(total) => format!("these {} items", locale.format_u64(total)),
        None => "these items".to_owned(),
    };
    match mode {
        DeleteMode::Permanent => format!(
            "Permanently delete {items} and folder contents? Links are removed without deleting their targets."
        ),
        DeleteMode::Trash => {
            format!("Move {items} to the system Trash? If unavailable, they will remain in place.")
        }
    }
}

fn count(value: u64, noun: &str, locale: &NumberLocale) -> String {
    format!(
        "{} {noun}{}",
        locale.format_u64(value),
        if value == 1 { "" } else { "s" }
    )
}

fn bytes(value: Option<u64>, locale: &NumberLocale) -> String {
    value.map_or_else(
        || "Unavailable".into(),
        |value| format!("{} bytes", locale.format_u64(value)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{EntryMetadata, MutationKind, OperationId};
    use std::path::PathBuf;

    fn preview(files: u64, folders: u64, sample: &[(&str, EntryKind)]) -> OperationPreview {
        OperationPreview {
            id: OperationId(1),
            kind: MutationKind::Delete(DeleteMode::Permanent),
            files,
            folders,
            file_bytes: Some(0),
            sample: sample
                .iter()
                .map(|(name, kind)| MutationItem {
                    path: PathBuf::from("parent").join(name),
                    kind: *kind,
                    metadata: EntryMetadata::default(),
                })
                .collect(),
        }
    }

    fn ask(preview: &OperationPreview, mode: DeleteMode) -> String {
        let locale = NumberLocale {
            group_separator: " ".into(),
            grouping: vec![3],
            repeat_last_group: true,
            ..NumberLocale::default()
        };
        question(preview, mode, &locale)
    }

    #[test]
    fn one_target_is_named_by_its_kind_and_the_warning_is_not_repeated() {
        use DeleteMode::{Permanent, Trash};
        for (files, folders, kind, permanent, trash) in [
            (
                0,
                1,
                EntryKind::Directory,
                "Permanently delete the folder “20261120-052223” and everything in it?",
                "Move the folder “20261120-052223” to the Trash? If the Trash is unavailable, it stays in place.",
            ),
            (
                1,
                0,
                EntryKind::File,
                "Permanently delete the file “20261120-052223”?",
                "Move the file “20261120-052223” to the Trash? If the Trash is unavailable, it stays in place.",
            ),
            (
                1,
                0,
                EntryKind::Symlink,
                "Permanently delete the link “20261120-052223”? What it points to is not deleted.",
                "Move the link “20261120-052223” to the Trash? If the Trash is unavailable, it stays in place.",
            ),
            // The worker counts a directory link with the folders; it is still a link.
            (
                0,
                1,
                EntryKind::DirectoryLink,
                "Permanently delete the link “20261120-052223”? What it points to is not deleted.",
                "Move the link “20261120-052223” to the Trash? If the Trash is unavailable, it stays in place.",
            ),
            (
                1,
                0,
                EntryKind::Other,
                "Permanently delete the item “20261120-052223”?",
                "Move the item “20261120-052223” to the Trash? If the Trash is unavailable, it stays in place.",
            ),
        ] {
            let one = preview(files, folders, &[("20261120-052223", kind)]);
            assert_eq!(ask(&one, Permanent), permanent);
            assert_eq!(ask(&one, Trash), trash);
            assert!(!ask(&one, Permanent).contains("cannot be undone"));
        }
        let link = preview(1, 0, &[("shortcut", EntryKind::Symlink)]);
        assert!(
            summary(&link, &NumberLocale::default()).starts_with("1 link\nName: shortcut\n"),
            "the summary names the same kind"
        );
    }

    #[test]
    fn several_targets_are_counted_without_names() {
        let sample = [("one.txt", EntryKind::File), ("two", EntryKind::Directory)];
        let several = preview(2_000, 3, &sample);
        assert_eq!(
            ask(&several, DeleteMode::Permanent),
            "Permanently delete these 2 003 items and folder contents? Links are removed without deleting their targets."
        );
        assert_eq!(
            ask(&several, DeleteMode::Trash),
            "Move these 2 003 items to the system Trash? If unavailable, they will remain in place."
        );
        // Two folders whose sample holds one: still counted, never named.
        let folders = preview(0, 2, &sample[1..]);
        assert!(ask(&folders, DeleteMode::Permanent).contains("these 2 items"));
        assert!(!ask(&folders, DeleteMode::Permanent).contains("“two”"));
        let unbounded = preview(u64::MAX, u64::MAX, &[]);
        assert!(
            ask(&unbounded, DeleteMode::Permanent).starts_with("Permanently delete these items ")
        );
    }

    #[test]
    fn a_long_name_is_kept_whole_for_the_dialog_to_wrap() {
        let long = format!("{}end", "folder-name-文档-👩🏽‍💻-".repeat(12));
        let one = preview(0, 1, &[(&long, EntryKind::Directory)]);
        assert_eq!(
            ask(&one, DeleteMode::Permanent),
            format!("Permanently delete the folder “{long}” and everything in it?")
        );
    }
}
