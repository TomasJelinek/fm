//! What a comparison badge says when it is hovered, and to a screen reader: the
//! comparison's verdict for that row, worded on demand from the compact detail the
//! marks store (nothing is kept per row). "Here" is the row's panel, "there" the
//! other one.
use super::*;
use midnight_core::{CompareDetail, CompareFlags, CompareInside, LocalizedDateTime};

/// The Windows attributes Compare directories compares, as their letters.
const ATTRIBUTE_LETTERS: [(u32, char); 8] = [
    (0x1, 'R'),
    (0x2, 'H'),
    (0x4, 'S'),
    (0x20, 'A'),
    (0x100, 'T'),
    (0x800, 'C'),
    (0x1000, 'O'),
    (0x4000, 'E'),
];

impl Presenter {
    /// The badge of `entry` in `pane`: 1 matches, 2 differs, 3 exists in this panel
    /// only, 4 not verified, 0 nothing to show (another listing, or not compared);
    /// with what it means for this row. `metadata` is the row's, as it is shown.
    pub(crate) fn comparison_badge(
        &self,
        pane: PaneId,
        entry: &Entry,
        metadata: &EntryMetadata,
    ) -> (i32, String) {
        let Some(shown) = &self.comparison.marks else {
            return (0, String::new());
        };
        let side = pane.index();
        if self.panes[side].generation != shown.generations[side] || entry.virtual_node.is_some() {
            return (0, String::new());
        }
        let Some((mark, detail)) = shown.marks.outcome(side, &entry.path) else {
            return (0, String::new());
        };
        let there = (detail.times.is_some())
            .then(|| self.comparison_counterpart(side, &shown.marks, &entry.path))
            .flatten();
        let row = Row {
            side,
            directory: matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink),
            options: &shown.options,
            here: shown_time(metadata),
            there,
            locale: &self.number_locale,
        };
        (mark_code(mark), describe(mark, &detail, &row))
    }

    /// The shown modification time of the entry compared with `path` (panel `side`),
    /// when the other panel lists it.
    fn comparison_counterpart(
        &self,
        side: usize,
        marks: &CompareMarks,
        path: &Path,
    ) -> Option<(i64, String)> {
        let relative = path.strip_prefix(&marks.roots()[side]).ok()?;
        let other = &self.panes[1 - side];
        let path = marks.roots()[1 - side].join(relative);
        let index = other.display_index();
        let row = *index.by_id.get(index.by_path.get(&path)?)?;
        shown_time(&other.display_entry(row)?.metadata)
    }
}

pub(super) fn mark_code(mark: CompareMark) -> i32 {
    match mark {
        CompareMark::Same => 1,
        CompareMark::Differs => 2,
        CompareMark::OnlyHere => 3,
        CompareMark::Unverified => 4,
    }
}

/// A row's modification time in whole seconds with its shown text: the panel's
/// own localized text, or the explicitly labelled UTC fallback.
fn shown_time(metadata: &EntryMetadata) -> Option<(i64, String)> {
    let modified = metadata.modified?;
    let seconds = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => i64::try_from(value.as_secs()).ok()?,
        Err(error) => {
            let value = error.duration();
            -i64::try_from(value.as_secs()).ok()? - i64::from(value.subsec_nanos() != 0)
        }
    };
    let LocalizedDateTime { date, time } = crate::time_presentation::modified(metadata)?;
    Some((seconds, format!("{date} {time}")))
}

/// What one row's badge describes it with.
pub(super) struct Row<'a> {
    /// 0 left, 1 right: the side "here" is.
    pub(super) side: usize,
    pub(super) directory: bool,
    pub(super) options: &'a CompareOptions,
    /// This row's and the other panel's entry's time, as whole seconds and shown.
    pub(super) here: Option<(i64, String)>,
    pub(super) there: Option<(i64, String)>,
    pub(super) locale: &'a NumberLocale,
}

/// The badge's text: one line per reason, the most telling first.
pub(super) fn describe(mark: CompareMark, detail: &CompareDetail, row: &Row) -> String {
    let flags = detail.flags;
    let mut lines = Vec::new();
    match mark {
        CompareMark::Same => lines.push(matches(row)),
        CompareMark::OnlyHere => lines.push(if row.directory {
            "Only in this panel, with everything inside".to_owned()
        } else {
            "Only in this panel".to_owned()
        }),
        CompareMark::Unverified => lines.push(format!("Not verified: {}", unverified(flags, row))),
        CompareMark::Differs => (),
    }
    if flags.contains(CompareFlags::CASE) {
        lines.push("Several names here or there differ only in case: they cannot be paired".into());
    }
    if flags.contains(CompareFlags::KIND) {
        lines.push("A file, a folder or a link opposite another kind: not compared".into());
    }
    if let Some(sizes) = detail.sizes {
        lines.push(sizes_line(sizes, row));
    }
    if let Some(times) = detail.times {
        lines.push(times_line(times, flags, row));
    }
    if let Some(attributes) = detail.attributes {
        lines.push(attributes_line(attributes, flags, row));
    }
    if let Some(offset) = detail.first_difference {
        lines.push(format!(
            "Content differs (first difference at byte {})",
            row.locale.format_u64(offset)
        ));
    }
    if let Some(inside) = detail.inside {
        lines.push(inside_line(&inside, row));
    }
    if lines.is_empty() {
        lines.push(if row.directory {
            "Something inside differs".to_owned()
        } else {
            "Differs".to_owned()
        });
    }
    lines.join("\n")
}

/// "Matches: same size and time (within 2 s)", by the criteria compared.
fn matches(row: &Row) -> String {
    let options = row.options;
    let mut criteria = Vec::new();
    if options.by_size {
        criteria.push("size".to_owned());
    }
    if options.by_modified {
        criteria.push(match options.time_tolerance_seconds {
            0 => "time".to_owned(),
            tolerance => format!(
                "time (within {} s)",
                row.locale.format_u64(tolerance.into())
            ),
        });
    }
    if options.by_attributes {
        criteria.push("attributes".to_owned());
    }
    if options.by_content {
        criteria.push("contents".to_owned());
    }
    let criteria = match criteria.split_last() {
        None => return "Matches: the same name in both panels".to_owned(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    };
    if row.directory {
        format!("Matches: everything inside has the same names, {criteria}")
    } else {
        format!("Matches: same {criteria}")
    }
}

fn unverified(flags: CompareFlags, row: &Row) -> &'static str {
    if flags.contains(CompareFlags::CHANGED) {
        "the file changed while it was compared; compare again"
    } else if flags.contains(CompareFlags::METADATA) {
        "the size or modification time is unavailable"
    } else if flags.contains(CompareFlags::LIMIT) {
        "the comparison reached its depth or memory limit here"
    } else if flags.contains(CompareFlags::UNREADABLE) {
        if row.directory {
            "the folder could not be listed completely"
        } else {
            "it could not be read"
        }
    } else {
        "it could not be compared completely"
    }
}

fn sizes_line(sizes: [u64; 2], row: &Row) -> String {
    let (here, there) = (sizes[row.side], sizes[1 - row.side]);
    format!(
        "{} here: {} vs {} bytes",
        if here > there { "Larger" } else { "Smaller" },
        row.locale.format_u64(here),
        row.locale.format_u64(there)
    )
}

/// "Newer here: <here> vs <there> (2 d)", with the panels' own time texts when both
/// rows are listed and still have the compared times; otherwise the difference only.
fn times_line(times: [i64; 2], flags: CompareFlags, row: &Row) -> String {
    let (here, there) = (times[row.side], times[1 - row.side]);
    let span = span(here.abs_diff(there));
    let newer = if here > there { "Newer" } else { "Older" };
    if flags.contains(CompareFlags::DST_SHIFT) {
        return if flags.contains(CompareFlags::TIME) {
            format!(
                "Daylight saving shift of {span}: {} here (counted as a difference; the Compare directories dialog can ignore it)",
                newer.to_lowercase()
            )
        } else {
            format!("Daylight saving shift of {span} (ignored)")
        };
    }
    let shown = |time: &Option<(i64, String)>, seconds| {
        time.as_ref()
            .filter(|(value, _)| *value == seconds)
            .map(|(_, text)| text.clone())
    };
    match (shown(&row.here, here), shown(&row.there, there)) {
        (Some(here), Some(there)) => format!("{newer} here: {here} vs {there} ({span})"),
        (Some(here), None) => format!("{newer} here by {span}: {here}"),
        _ => format!("{newer} here by {span}"),
    }
}

fn attributes_line(attributes: [u32; 2], flags: CompareFlags, row: &Row) -> String {
    let (here, there) = (attributes[row.side], attributes[1 - row.side]);
    if flags.contains(CompareFlags::MODE) {
        let what = if row.directory {
            "Folder permissions"
        } else {
            "Permissions"
        };
        return format!("{what} differ: here {here:04o}, there {there:04o}");
    }
    let letters = |value: u32| {
        let text: String = ATTRIBUTE_LETTERS
            .iter()
            .filter(|(bit, _)| value & bit != 0)
            .map(|(_, letter)| *letter)
            .collect();
        if text.is_empty() {
            "—".to_owned()
        } else {
            text
        }
    };
    let what = if row.directory {
        "Folder attributes"
    } else {
        "Attributes"
    };
    format!(
        "{what} differ: here {}, there {}",
        letters(here),
        letters(there)
    )
}

/// "Differs inside: 3 files and 1 folder differ, 2 only here, 1 only there".
fn inside_line(inside: &CompareInside, row: &Row) -> String {
    let count = |value: u32| row.locale.format_u64(value.into());
    let mut parts = Vec::new();
    let mut differ = Vec::new();
    for (value, one, many) in [
        (inside.files, "file", "files"),
        (inside.folders, "folder", "folders"),
    ] {
        if value > 0 {
            differ.push(format!(
                "{} {}",
                count(value),
                if value == 1 { one } else { many }
            ));
        }
    }
    if !differ.is_empty() {
        let total = u64::from(inside.files) + u64::from(inside.folders);
        parts.push(format!(
            "{} {}",
            differ.join(" and "),
            if total == 1 { "differs" } else { "differ" }
        ));
    }
    let (here, there) = (inside.only[row.side], inside.only[1 - row.side]);
    if here > 0 {
        parts.push(format!("{} only here", count(here)));
    }
    if there > 0 {
        parts.push(format!("{} only in the other panel", count(there)));
    }
    if inside.unverified > 0 {
        parts.push(format!("{} not verified", count(inside.unverified)));
    }
    format!("Differs inside: {}", parts.join(", "))
}

/// A time difference in its two largest units: "2 d", "1 h 5 min", "45 s".
fn span(seconds: u64) -> String {
    const UNITS: [(u64, &str); 4] = [(86_400, "d"), (3_600, "h"), (60, "min"), (1, "s")];
    let Some(first) = UNITS.iter().position(|(size, _)| seconds >= *size) else {
        return "0 s".to_owned();
    };
    let (size, name) = UNITS[first];
    let mut text = format!("{} {name}", seconds / size);
    if let Some(&(next, next_name)) = UNITS.get(first + 1) {
        let value = seconds % size / next;
        if value > 0 {
            text.push_str(&format!(" {value} {next_name}"));
        }
    }
    text
}

#[cfg(test)]
pub(super) fn span_for_tests(seconds: u64) -> String {
    span(seconds)
}
