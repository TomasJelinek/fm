//! Constant-size delete presentation; the worker owns and summarizes the full target list.
use midnight_core::{EntryKind, NumberLocale, OperationPreview};

pub(super) fn summary(preview: &OperationPreview, locale: &NumberLocale) -> String {
    if preview.files == 1
        && preview.folders == 0
        && let Some(item) = preview.sample.first()
    {
        let name = item
            .path
            .file_name()
            .unwrap_or(item.path.as_os_str())
            .to_string_lossy();
        let modified = crate::time_presentation::modified(&item.metadata)
            .map(|value| format!("{} {}", value.date, value.time))
            .unwrap_or_else(|| "Unavailable".into());
        let size = (item.kind == EntryKind::File)
            .then_some(item.metadata.size)
            .flatten();
        return format!(
            "1 file\nName: {name}\nSize: {}\nModified: {modified}",
            bytes(size, locale)
        );
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
