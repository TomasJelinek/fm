//! Constant-size delete presentation; the worker owns and summarizes the full target list.
//! The confirmation asks its question first, says right under it what cannot be undone,
//! and lists the rest as details: what is deleted, how large it is, when it changed and
//! where it is. Everything comes from the worker's preview and sizes the panels already
//! measured; nothing here reads the filesystem.
use midnight_core::{
    DeleteMode, EntryKind, FolderSize, MutationItem, NumberLocale, OperationPreview,
};
use std::path::{Path, PathBuf};

/// How many names a confirmation of several items lists before "and N more". One name
/// more is listed instead of "and 1 more".
const NAMES: usize = 5;

/// What a Delete confirmation shows, top to bottom.
pub(crate) struct Confirmation {
    /// The headline: what is deleted, by name for one target, counted for several.
    pub(crate) question: String,
    /// Right under the question: that a permanent delete cannot be undone, or what
    /// happens to the items where there is no Trash.
    pub(crate) warning: &'static str,
    /// Label and value; a value of several lines lists names, one per line.
    pub(crate) details: Vec<(&'static str, String)>,
    /// The detail whose value is a folder's path: the dialog shortens it in the middle,
    /// as the path bar does, instead of wrapping it.
    pub(crate) path: Option<usize>,
    /// That folder, which the dialog splits into the path bar's components.
    pub(crate) folder: Option<PathBuf>,
}

impl Confirmation {
    /// The confirmation as one text: the question, then a "Label: value" line per detail
    /// (the warning is the dialog's own line). Tests, the acceptance harness and the
    /// shared form's body read it.
    pub(crate) fn text(&self) -> String {
        let mut text = self.question.clone();
        text.push('\n');
        for (label, value) in &self.details {
            text.push_str(&format!("\n{label}: {}", value.replace('\n', "\n  ")));
        }
        text
    }

    /// Every label, one per line: the dialog's label column is as wide as the widest.
    pub(crate) fn labels(&self) -> String {
        let labels: Vec<_> = self.details.iter().map(|(label, _)| *label).collect();
        labels.join("\n")
    }
}

/// Where the confirmation finds a measured folder size and the panel's folder.
pub(crate) struct Context<'a> {
    pub(crate) locale: &'a NumberLocale,
    /// The size Folder sizes measured for this folder in a panel, if any.
    pub(crate) folder_size: &'a dyn Fn(&Path) -> Option<FolderSize>,
    /// The folder of the panel the delete was asked from.
    pub(crate) source: Option<&'a Path>,
}

pub(crate) fn confirmation(
    preview: &OperationPreview,
    mode: DeleteMode,
    context: &Context<'_>,
) -> Confirmation {
    match single(preview) {
        Some(item) => one(item, mode, context),
        None => several(preview, mode, context),
    }
}

/// The single target, when the confirmation deletes exactly one item.
fn single(preview: &OperationPreview) -> Option<&MutationItem> {
    (preview.files.checked_add(preview.folders) == Some(1))
        .then(|| preview.sample.first())
        .flatten()
}

fn name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

fn warning(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::Permanent => "Permanent deletion cannot be undone.",
        DeleteMode::Trash => {
            "If the Trash is unavailable, permanent deletion requires a separate confirmation."
        }
    }
}

/// One target, named in the question by its kind. The worker counts directory links
/// with folders and other links with files; both are links here.
fn one(item: &MutationItem, mode: DeleteMode, context: &Context<'_>) -> Confirmation {
    let name = name(&item.path);
    let link = matches!(item.kind, EntryKind::DirectoryLink | EntryKind::Symlink);
    let question = match (mode, item.kind) {
        (DeleteMode::Permanent, EntryKind::Directory) => {
            format!("Permanently delete the folder “{name}” and everything in it?")
        }
        (DeleteMode::Permanent, _) if link => format!("Permanently delete the link “{name}”?"),
        (DeleteMode::Permanent, _) => format!("Permanently delete “{name}”?"),
        (DeleteMode::Trash, EntryKind::Directory) => {
            format!("Move the folder “{name}” to the Trash?")
        }
        (DeleteMode::Trash, _) if link => format!("Move the link “{name}” to the Trash?"),
        (DeleteMode::Trash, _) => format!("Move “{name}” to the Trash?"),
    };
    let kind = match item.kind {
        EntryKind::File => "File",
        EntryKind::Directory => "Folder",
        EntryKind::DirectoryLink => "Folder link (the folder it points to is kept)",
        EntryKind::Symlink => "Link (what it points to is kept)",
        EntryKind::Other => "Special file",
    };
    let mut details = vec![("Name", name.into_owned()), ("Type", kind.to_owned())];
    match item.kind {
        EntryKind::File => details.push(("Size", size_or_unavailable(item.metadata.size, context))),
        EntryKind::Directory => {
            if let Some(size) = (context.folder_size)(&item.path) {
                details.push(("Size", folder_size(&size, context.locale)));
            }
        }
        _ => {}
    }
    let modified = crate::time_presentation::modified(&item.metadata)
        .map(|value| format!("{} {}", value.date, value.time))
        .unwrap_or_else(|| "Unavailable".into());
    details.push(("Modified", modified));
    let (mut path, mut folder) = (None, None);
    if let Some(parent) = item
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        path = Some(details.len());
        details.push(("Location", parent.display().to_string()));
        folder = Some(parent.to_path_buf());
    }
    Confirmation {
        question,
        warning: warning(mode),
        details,
        path,
        folder,
    }
}

/// Several targets, counted in the question; the details count files and folders, list
/// the first names and add up the sizes that are known.
fn several(preview: &OperationPreview, mode: DeleteMode, context: &Context<'_>) -> Confirmation {
    let locale = context.locale;
    let total = preview.files.checked_add(preview.folders);
    let items = total.map_or_else(
        || "these items".to_owned(),
        |total| count(total, "item", locale),
    );
    let question = match mode {
        DeleteMode::Permanent => format!("Permanently delete {items}?"),
        DeleteMode::Trash => format!("Move {items} to the Trash?"),
    };
    let mut counted = String::new();
    if preview.files != 0 || preview.folders == 0 {
        counted = count(preview.files, "file", locale);
    }
    if preview.folders != 0 {
        if !counted.is_empty() {
            counted.push_str(" and ");
        }
        counted.push_str(&count(preview.folders, "folder", locale));
        counted.push_str(if preview.folders == 1 {
            ", including everything in the folder"
        } else {
            ", including everything in the folders"
        });
    }
    let mut details = vec![("Items", counted)];
    // The whole selection is in the sample when it is no larger than the sample.
    let complete = total == Some(preview.sample.len() as u64);
    if preview
        .sample
        .iter()
        .any(|item| matches!(item.kind, EntryKind::DirectoryLink | EntryKind::Symlink))
    {
        details.push((
            "Links",
            "Only the links go; what they point to is kept".into(),
        ));
    }
    if !preview.sample.is_empty() {
        details.push(("Names", names(preview, total, locale)));
    }
    if let Some(size) = several_size(preview, complete, context) {
        details.push(("Size", size));
    }
    let (mut path, mut folder) = (None, None);
    match location(preview, complete, context.source) {
        Some(Location::Folder(parent)) => {
            path = Some(details.len());
            details.push(("Location", parent.display().to_string()));
            folder = Some(parent);
        }
        Some(Location::Several) => details.push(("Location", "Several folders".into())),
        None => {}
    }
    Confirmation {
        question,
        warning: warning(mode),
        details,
        path,
        folder,
    }
}

/// The first names, one per line, and how many more there are.
fn names(preview: &OperationPreview, total: Option<u64>, locale: &NumberLocale) -> String {
    let listed = match total {
        Some(total) if total <= NAMES as u64 + 1 => preview.sample.len(),
        _ => NAMES.min(preview.sample.len()),
    };
    let mut lines: Vec<String> = preview.sample[..listed]
        .iter()
        .map(|item| name(&item.path).into_owned())
        .collect();
    match total.map(|total| total.saturating_sub(listed as u64)) {
        Some(0) => {}
        Some(more) => lines.push(format!("and {} more", locale.format_u64(more))),
        None => lines.push("and more".into()),
    }
    lines.join("\n")
}

/// The files' sizes, with the folders' when every folder was measured; otherwise what
/// the size leaves out. None when nothing is known (folders only, none measured).
fn several_size(
    preview: &OperationPreview,
    complete: bool,
    context: &Context<'_>,
) -> Option<String> {
    let locale = context.locale;
    if preview.folders == 0 {
        return Some(size_or_unavailable(preview.file_bytes, context));
    }
    // Folder links count with the folders; deleting one deletes nothing inside it.
    let folders = complete
        .then(|| {
            preview
                .sample
                .iter()
                .filter(|item| item.kind == EntryKind::Directory)
                .map(|item| (context.folder_size)(&item.path))
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    match (preview.file_bytes, folders) {
        (Some(files), Some(folders)) => {
            let bytes = folders
                .iter()
                .try_fold(files, |sum, folder| sum.checked_add(folder.bytes))?;
            let text = size(bytes, locale);
            Some(if folders.iter().all(final_size) {
                text
            } else {
                format!("At least {text} (folder sizes not final)")
            })
        }
        (Some(files), None) if preview.files != 0 => Some(format!(
            "{} in the files; folder contents not counted",
            size(files, locale)
        )),
        (None, _) if preview.files != 0 => Some("Unavailable".into()),
        _ => None,
    }
}

enum Location {
    Folder(PathBuf),
    Several,
}

/// The folder all items are in, or several when the sample shows more than one. A
/// selection larger than the sample is in the panel's folder only when the whole sample
/// is.
fn location(preview: &OperationPreview, complete: bool, source: Option<&Path>) -> Option<Location> {
    let mut parents = preview.sample.iter().map(|item| item.path.parent());
    let first = parents.next()??;
    if parents.any(|parent| parent != Some(first)) {
        return Some(Location::Several);
    }
    (!first.as_os_str().is_empty() && (complete || source == Some(first)))
        .then(|| Location::Folder(first.to_path_buf()))
}

fn final_size(size: &FolderSize) -> bool {
    size.finished && size.errors == 0
}

fn folder_size(size: &FolderSize, locale: &NumberLocale) -> String {
    let text = format!(
        "{} in {}",
        self::size(size.bytes, locale),
        count(size.files, "file", locale)
    );
    if final_size(size) {
        text
    } else {
        format!("At least {text} (not final)")
    }
}

fn size_or_unavailable(value: Option<u64>, context: &Context<'_>) -> String {
    value.map_or_else(|| "Unavailable".into(), |value| size(value, context.locale))
}

/// The readable size as the panels show it, with the exact bytes: "24.1 MiB
/// (25 320 511 bytes)"; below 1 KiB the bytes alone.
fn size(bytes: u64, locale: &NumberLocale) -> String {
    let exact = count(bytes, "byte", locale);
    if bytes < 1024 {
        exact
    } else {
        format!("{} ({exact})", crate::format_size(bytes, locale))
    }
}

fn count(value: u64, noun: &str, locale: &NumberLocale) -> String {
    format!(
        "{} {noun}{}",
        locale.format_u64(value),
        if value == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{EntryMetadata, LocalizedDateTime, MutationKind, OperationId};
    use std::path::PathBuf;

    fn folder() -> PathBuf {
        PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("data")
    }

    fn item(name: &str, kind: EntryKind, size: Option<u64>) -> MutationItem {
        MutationItem {
            path: folder().join(name),
            kind,
            metadata: EntryMetadata {
                size,
                localized_modified: Some(LocalizedDateTime {
                    date: "23.11.2026".into(),
                    time: "14:53:51".into(),
                }),
                ..EntryMetadata::default()
            },
        }
    }

    fn preview(
        files: u64,
        folders: u64,
        bytes: Option<u64>,
        sample: Vec<MutationItem>,
    ) -> OperationPreview {
        OperationPreview {
            id: OperationId(1),
            kind: MutationKind::Delete(DeleteMode::Permanent),
            files,
            folders,
            file_bytes: bytes,
            sample,
        }
    }

    fn locale() -> NumberLocale {
        NumberLocale {
            decimal_separator: ",".into(),
            group_separator: " ".into(),
            grouping: vec![3],
            repeat_last_group: true,
        }
    }

    fn confirm(preview: &OperationPreview, mode: DeleteMode) -> Confirmation {
        confirm_with(preview, mode, &|_| None, None)
    }

    fn confirm_with(
        preview: &OperationPreview,
        mode: DeleteMode,
        sizes: &dyn Fn(&Path) -> Option<FolderSize>,
        source: Option<&Path>,
    ) -> Confirmation {
        let locale = locale();
        let context = Context {
            locale: &locale,
            folder_size: sizes,
            source,
        };
        confirmation(preview, mode, &context)
    }

    fn detail<'a>(confirmation: &'a Confirmation, label: &str) -> Option<&'a str> {
        confirmation
            .details
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, value)| value.as_str())
    }

    fn labels(confirmation: &Confirmation) -> Vec<&'static str> {
        confirmation
            .details
            .iter()
            .map(|(label, _)| *label)
            .collect()
    }

    #[test]
    fn one_file_asks_by_name_first_and_lists_its_details() {
        let one = preview(
            1,
            0,
            Some(25_320_511),
            vec![item("experiments.rar", EntryKind::File, Some(25_320_511))],
        );
        let permanent = confirm(&one, DeleteMode::Permanent);
        assert_eq!(permanent.question, "Permanently delete “experiments.rar”?");
        assert_eq!(permanent.warning, "Permanent deletion cannot be undone.");
        assert_eq!(
            labels(&permanent),
            ["Name", "Type", "Size", "Modified", "Location"]
        );
        assert_eq!(detail(&permanent, "Name"), Some("experiments.rar"));
        assert_eq!(detail(&permanent, "Type"), Some("File"));
        assert_eq!(
            detail(&permanent, "Size"),
            Some("24,1 MiB (25 320 511 bytes)")
        );
        assert_eq!(detail(&permanent, "Modified"), Some("23.11.2026 14:53:51"));
        assert_eq!(
            detail(&permanent, "Location"),
            Some(folder().display().to_string().as_str())
        );
        // The question comes first in the text too; the warning is the dialog's own line.
        assert!(
            permanent.text().starts_with(
                "Permanently delete “experiments.rar”?\n\nName: experiments.rar\nType: File\nSize: 24,1 MiB (25 320 511 bytes)\n"
            ),
            "{}",
            permanent.text()
        );
        assert!(!permanent.text().contains("cannot be undone"));
        assert_eq!(permanent.labels(), "Name\nType\nSize\nModified\nLocation");
        assert_eq!(
            permanent.path,
            Some(4),
            "the folder's path is shortened, not wrapped"
        );
        assert_eq!(permanent.folder, Some(folder()));

        let trash = confirm(&one, DeleteMode::Trash);
        assert_eq!(trash.question, "Move “experiments.rar” to the Trash?");
        assert_eq!(
            trash.warning,
            "If the Trash is unavailable, permanent deletion requires a separate confirmation."
        );
        assert_eq!(trash.details, permanent.details);
    }

    #[test]
    fn small_unknown_and_missing_values_are_said_plainly() {
        let mut small = item("a.txt", EntryKind::File, Some(1));
        small.metadata.localized_modified = None;
        let one = preview(1, 0, Some(1), vec![small.clone()]);
        let shown = confirm(&one, DeleteMode::Permanent);
        assert_eq!(detail(&shown, "Size"), Some("1 byte"));
        assert_eq!(detail(&shown, "Modified"), Some("Unavailable"));
        small.metadata.size = Some(1_023);
        let one = preview(1, 0, Some(1_023), vec![small.clone()]);
        assert_eq!(
            detail(&confirm(&one, DeleteMode::Permanent), "Size"),
            Some("1 023 bytes")
        );
        small.metadata.size = None;
        let one = preview(1, 0, None, vec![small]);
        assert_eq!(
            detail(&confirm(&one, DeleteMode::Permanent), "Size"),
            Some("Unavailable")
        );
    }

    #[test]
    fn one_folder_or_link_is_named_by_its_kind() {
        use DeleteMode::{Permanent, Trash};
        for (files, folders, kind, permanent, trash, type_text) in [
            (
                0,
                1,
                EntryKind::Directory,
                "Permanently delete the folder “20261120-052223” and everything in it?",
                "Move the folder “20261120-052223” to the Trash?",
                "Folder",
            ),
            (
                1,
                0,
                EntryKind::Symlink,
                "Permanently delete the link “20261120-052223”?",
                "Move the link “20261120-052223” to the Trash?",
                "Link (what it points to is kept)",
            ),
            // The worker counts a directory link with the folders; it is still a link.
            (
                0,
                1,
                EntryKind::DirectoryLink,
                "Permanently delete the link “20261120-052223”?",
                "Move the link “20261120-052223” to the Trash?",
                "Folder link (the folder it points to is kept)",
            ),
            (
                1,
                0,
                EntryKind::Other,
                "Permanently delete “20261120-052223”?",
                "Move “20261120-052223” to the Trash?",
                "Special file",
            ),
        ] {
            let one = preview(
                files,
                folders,
                Some(0),
                vec![item("20261120-052223", kind, Some(9))],
            );
            let shown = confirm(&one, Permanent);
            assert_eq!(shown.question, permanent);
            assert_eq!(confirm(&one, Trash).question, trash);
            assert_eq!(detail(&shown, "Type"), Some(type_text));
            // Only a file has a size of its own; a folder's is its measured contents.
            assert_eq!(detail(&shown, "Size"), None, "{kind:?}");
            assert!(!shown.question.contains("cannot be undone"));
        }
    }

    #[test]
    fn a_measured_folder_shows_its_size_and_whether_it_is_final() {
        let one = preview(
            0,
            1,
            Some(0),
            vec![item("photos", EntryKind::Directory, None)],
        );
        let measured = |finished: bool| {
            move |path: &Path| {
                (path == folder().join("photos")).then(|| FolderSize {
                    bytes: 3 * 1024 * 1024,
                    files: 12,
                    finished,
                    ..FolderSize::default()
                })
            }
        };
        let shown = confirm_with(&one, DeleteMode::Permanent, &measured(true), None);
        assert_eq!(
            detail(&shown, "Size"),
            Some("3,0 MiB (3 145 728 bytes) in 12 files")
        );
        let shown = confirm_with(&one, DeleteMode::Permanent, &measured(false), None);
        assert_eq!(
            detail(&shown, "Size"),
            Some("At least 3,0 MiB (3 145 728 bytes) in 12 files (not final)")
        );
    }

    fn mixed() -> Vec<MutationItem> {
        vec![
            item("one.txt", EntryKind::File, Some(1_000)),
            item("two", EntryKind::Directory, None),
            item("three.txt", EntryKind::File, Some(2_000)),
            item("four", EntryKind::Directory, None),
            item("five.txt", EntryKind::File, Some(48)),
            item("six.txt", EntryKind::File, Some(0)),
            item("seven.txt", EntryKind::File, Some(0)),
        ]
    }

    #[test]
    fn several_targets_are_counted_and_their_first_names_listed() {
        let several = preview(2_000, 3, Some(3_048), mixed());
        let permanent = confirm(&several, DeleteMode::Permanent);
        assert_eq!(permanent.question, "Permanently delete 2 003 items?");
        assert_eq!(permanent.warning, "Permanent deletion cannot be undone.");
        assert_eq!(labels(&permanent), ["Items", "Names", "Size"]);
        assert_eq!(
            detail(&permanent, "Items"),
            Some("2 000 files and 3 folders, including everything in the folders")
        );
        assert_eq!(
            detail(&permanent, "Names"),
            Some("one.txt\ntwo\nthree.txt\nfour\nfive.txt\nand 1 998 more")
        );
        assert_eq!(
            detail(&permanent, "Size"),
            Some("3,0 KiB (3 048 bytes) in the files; folder contents not counted")
        );
        assert!(
            permanent
                .text()
                .contains("\nNames: one.txt\n  two\n  three.txt\n"),
            "{}",
            permanent.text()
        );
        let trash = confirm(&several, DeleteMode::Trash);
        assert_eq!(trash.question, "Move 2 003 items to the Trash?");
        assert_eq!(
            trash.warning,
            "If the Trash is unavailable, permanent deletion requires a separate confirmation."
        );
        // The selection is larger than the sample: its folder is known from the panel.
        let source = folder();
        let shown = confirm_with(&several, DeleteMode::Trash, &|_| None, Some(&source));
        assert_eq!(
            detail(&shown, "Location"),
            Some(source.display().to_string().as_str())
        );
        let elsewhere = folder().join("elsewhere");
        let shown = confirm_with(&several, DeleteMode::Trash, &|_| None, Some(&elsewhere));
        assert_eq!(detail(&shown, "Location"), None);
    }

    #[test]
    fn a_few_targets_are_all_named_with_their_folder() {
        let six: Vec<_> = mixed().into_iter().take(6).collect();
        let few = preview(4, 2, Some(3_048), six);
        let shown = confirm(&few, DeleteMode::Permanent);
        assert_eq!(shown.question, "Permanently delete 6 items?");
        assert_eq!(
            detail(&shown, "Names"),
            Some("one.txt\ntwo\nthree.txt\nfour\nfive.txt\nsix.txt"),
            "six names fit where five and one more would"
        );
        assert_eq!(
            detail(&shown, "Location"),
            Some(folder().display().to_string().as_str())
        );
        // Every folder measured: the size is the total.
        let total = confirm_with(
            &few,
            DeleteMode::Permanent,
            &|_| {
                Some(FolderSize {
                    bytes: 1_000_000,
                    files: 3,
                    finished: true,
                    ..FolderSize::default()
                })
            },
            None,
        );
        assert_eq!(detail(&total, "Size"), Some("1,9 MiB (2 003 048 bytes)"));
        let partial = confirm_with(
            &few,
            DeleteMode::Permanent,
            &|path| {
                Some(FolderSize {
                    bytes: 1_000_000,
                    files: 3,
                    finished: path.ends_with("two"),
                    ..FolderSize::default()
                })
            },
            None,
        );
        assert_eq!(
            detail(&partial, "Size"),
            Some("At least 1,9 MiB (2 003 048 bytes) (folder sizes not final)")
        );
        // Folders only, none measured: no size is better than a wrong one.
        let folders = preview(
            0,
            2,
            Some(0),
            vec![
                item("two", EntryKind::Directory, None),
                item("four", EntryKind::Directory, None),
            ],
        );
        let shown = confirm(&folders, DeleteMode::Permanent);
        assert_eq!(
            detail(&shown, "Items"),
            Some("2 folders, including everything in the folders")
        );
        assert_eq!(detail(&shown, "Size"), None);
        assert_eq!(detail(&shown, "Names"), Some("two\nfour"));
    }

    #[test]
    fn links_and_several_folders_are_said() {
        let mut sample = mixed();
        sample.truncate(2);
        sample.push(item("shortcut", EntryKind::Symlink, None));
        sample[1].path = folder().join("other").join("two");
        let several = preview(2, 1, Some(1_000), sample);
        let shown = confirm(&several, DeleteMode::Permanent);
        assert_eq!(
            detail(&shown, "Links"),
            Some("Only the links go; what they point to is kept")
        );
        assert_eq!(detail(&shown, "Location"), Some("Several folders"));
        assert_eq!(shown.path, None, "Several folders is no path");
        assert_eq!(shown.folder, None);
        assert_eq!(
            detail(&shown, "Items"),
            Some("2 files and 1 folder, including everything in the folder")
        );
    }

    #[test]
    fn counts_that_overflow_still_ask_about_every_item() {
        let unbounded = preview(u64::MAX, u64::MAX, None, Vec::new());
        let shown = confirm(&unbounded, DeleteMode::Permanent);
        assert_eq!(shown.question, "Permanently delete these items?");
        assert_eq!(
            detail(&shown, "Items"),
            Some(
                "18 446 744 073 709 551 615 files and 18 446 744 073 709 551 615 folders, including everything in the folders"
            )
        );
        assert_eq!(detail(&shown, "Names"), None);
        assert_eq!(detail(&shown, "Size"), Some("Unavailable"));
        let sampled = preview(u64::MAX, 1, None, mixed());
        assert!(
            detail(&confirm(&sampled, DeleteMode::Trash), "Names")
                .unwrap()
                .ends_with("\nand more")
        );
    }

    #[test]
    fn a_long_name_is_kept_whole_for_the_dialog_to_wrap() {
        let long = format!("{}end", "folder-name-文档-👩🏽‍💻-".repeat(12));
        let one = preview(0, 1, Some(0), vec![item(&long, EntryKind::Directory, None)]);
        let shown = confirm(&one, DeleteMode::Permanent);
        assert_eq!(
            shown.question,
            format!("Permanently delete the folder “{long}” and everything in it?")
        );
        assert_eq!(detail(&shown, "Name"), Some(long.as_str()));
    }
}
