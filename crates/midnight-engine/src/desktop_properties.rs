//! The read-only metadata summary shown when the platform opens no Properties
//! dialog (Linux, and any provider without one). Windows opens its own dialog.
//! A single folder's contents are counted afterwards on a backend worker.

use midnight_core::{
    Cancellation, DesktopActionResult, EntryMetadata, FsError, NumberLocale, PropertyAccount,
    PropertyDetails,
};
use std::{
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "desktop_properties_contents.rs"]
mod contents;
pub(super) use contents::{Totals, count as count_contents};

/// What one selected entry is, from its raw metadata. Folder contents are never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    File,
    Folder,
    Link,
    /// A device, pipe or socket, or an entry whose type was not reported.
    Other(&'static str),
}

impl Kind {
    fn of(metadata: &EntryMetadata) -> Self {
        if let Some(mode) = metadata.unix_mode {
            return match mode & 0o170_000 {
                0o100_000 => Self::File,
                0o040_000 => Self::Folder,
                0o120_000 => Self::Link,
                0o060_000 => Self::Other("Block device"),
                0o020_000 => Self::Other("Character device"),
                0o010_000 => Self::Other("Pipe"),
                0o140_000 => Self::Other("Socket"),
                _ => Self::Other("Item"),
            };
        }
        const WINDOWS_DIRECTORY: u32 = 0x10;
        match metadata.native_attributes {
            Some(attributes) if attributes & WINDOWS_DIRECTORY != 0 => Self::Folder,
            _ if metadata.size.is_some() => Self::File,
            _ => Self::Other("Item"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Folder => "Folder",
            Self::Link => "Link",
            Self::Other(name) => name,
        }
    }
}

#[derive(Default)]
struct Selection {
    files: u64,
    folders: u64,
    links: u64,
    others: u64,
    unreadable: u64,
    bytes: u64,
}

/// Native details of one entry, formatted on the worker for the summary text.
pub(super) struct Details {
    owner: Option<String>,
    group: Option<String>,
    created: Option<String>,
    modified: Option<String>,
    accessed: Option<String>,
    unix_mode: Option<u32>,
}

impl Details {
    /// Times use `time`, the user's date format; an unformattable time is omitted.
    pub(super) fn format(details: PropertyDetails, time: &dyn Fn(SystemTime) -> String) -> Self {
        let account = |account: Option<PropertyAccount>| {
            account.map(|account| match account.name {
                Some(name) => format!("{name} ({})", account.id),
                None => account.id.to_string(),
            })
        };
        Self {
            owner: account(details.owner),
            group: account(details.group),
            created: details.created.map(time),
            modified: details.modified.map(time),
            accessed: details.accessed.map(time),
            unix_mode: details.unix_mode,
        }
    }
}

/// The contents line of a single folder's summary.
pub(super) enum Contents<'a> {
    /// Still counting; the totals so far, if any.
    Calculating(Option<&'a Totals>),
    Counted(&'a Totals),
    Failed(&'a str),
}

/// The summary of one entry, kept to show its folder contents once counted.
pub(super) struct Single {
    pub(super) title: String,
    pub(super) path: PathBuf,
    metadata: EntryMetadata,
    details: Option<Details>,
    locale: NumberLocale,
}

pub(super) struct Summary {
    pub(super) result: DesktopActionResult,
    /// A single folder whose contents are counted next.
    pub(super) folder: Option<Single>,
}

/// Reads each selected entry's own metadata; a single unreadable entry is an error.
/// `details` adds a single entry's owner, group, times and permissions where the
/// platform reports them.
pub(super) fn summary(
    paths: &[PathBuf],
    locale: &NumberLocale,
    cancel: &Cancellation,
    entry_metadata: &dyn Fn(&Path) -> Result<EntryMetadata, FsError>,
    details: &dyn Fn(&Path) -> Option<Details>,
) -> Result<Summary, FsError> {
    if let [path] = paths {
        crate::captured_native::check_cancel(cancel)?;
        let metadata = entry_metadata(path)?;
        crate::captured_native::check_cancel(cancel)?;
        let details = details(path);
        crate::captured_native::check_cancel(cancel)?;
        let single = Single {
            title: path.display().to_string(),
            path: path.clone(),
            metadata,
            details,
            locale: locale.clone(),
        };
        let folder = Kind::of(&single.metadata) == Kind::Folder;
        let body = single.body(folder.then_some(Contents::Calculating(None)));
        return Ok(Summary {
            result: DesktopActionResult::PropertiesInfo {
                title: single.title.clone(),
                body,
            },
            folder: folder.then_some(single),
        });
    }
    let mut totals = Selection::default();
    for path in paths {
        crate::captured_native::check_cancel(cancel)?;
        let metadata = entry_metadata(path);
        crate::captured_native::check_cancel(cancel)?;
        let Ok(metadata) = metadata else {
            totals.unreadable += 1;
            continue;
        };
        match Kind::of(&metadata) {
            Kind::File => totals.files += 1,
            Kind::Folder => totals.folders += 1,
            Kind::Link => totals.links += 1,
            Kind::Other(_) => totals.others += 1,
        }
        if let Some(size) = metadata.size {
            totals.bytes = totals.bytes.checked_add(size).ok_or_else(|| {
                FsError::from_io(
                    "properties",
                    path,
                    io::Error::other("Combined size exceeds u64."),
                )
            })?;
        }
    }
    Ok(Summary {
        result: DesktopActionResult::PropertiesInfo {
            title: format!("{} selected items", locale.format_u64(paths.len() as u64)),
            body: several(&totals, locale),
        },
        folder: None,
    })
}

impl Single {
    /// The summary text; `contents` is the folder contents line, if any.
    pub(super) fn body(&self, contents: Option<Contents<'_>>) -> String {
        let locale = &self.locale;
        let kind = Kind::of(&self.metadata);
        let mut lines = vec![format!("Type: {}", kind.name())];
        if let Some(size) = self.metadata.size {
            lines.push(format!("Size: {}", count(locale, size, "byte", "bytes")));
        }
        // A link's own mode bits are not used; its target's permissions apply.
        let permissions = self
            .details
            .as_ref()
            .and_then(|details| details.unix_mode)
            .or(self.metadata.unix_mode)
            .filter(|_| kind != Kind::Link)
            .map(|mode| format!("Permissions: {}", permissions(mode)));
        match &self.details {
            Some(details) => {
                if let Some(owner) = &details.owner {
                    lines.push(format!("Owner: {owner}"));
                }
                if let Some(group) = &details.group {
                    lines.push(format!("Group: {group}"));
                }
                lines.extend(permissions);
                lines.push(format!(
                    "Created: {}",
                    details.created.as_deref().unwrap_or("not available")
                ));
                let modified = details.modified.clone().or_else(|| {
                    self.metadata
                        .localized_modified
                        .as_ref()
                        .map(|modified| format!("{} {}", modified.date, modified.time))
                });
                if let Some(modified) = modified {
                    lines.push(format!("Modified: {modified}"));
                }
                if let Some(accessed) = &details.accessed {
                    lines.push(format!("Accessed: {accessed}"));
                }
            }
            None => {
                if let Some(modified) = &self.metadata.localized_modified {
                    lines.push(format!("Modified: {} {}", modified.date, modified.time));
                }
                lines.extend(permissions);
            }
        }
        let listed = contents.is_some();
        match contents {
            None => {}
            Some(Contents::Calculating(None)) => lines.push("Contents: Calculating…".into()),
            Some(Contents::Calculating(Some(totals))) => lines.push(format!(
                "Contents: Calculating… {} so far",
                items(locale, totals)
            )),
            Some(Contents::Counted(totals)) => {
                lines.push(format!(
                    "Contents: {}, {}",
                    items(locale, totals),
                    count(locale, totals.bytes, "byte", "bytes")
                ));
                lines.extend(not_counted(locale, totals));
            }
            Some(Contents::Failed(reason)) => {
                lines.push(format!("Contents: could not be counted ({reason})"));
            }
        }
        lines.push(String::new());
        lines.push(note(kind == Kind::Folder && !listed).into());
        lines.join("\n")
    }
}

fn items(locale: &NumberLocale, totals: &Totals) -> String {
    format!(
        "{}, {}",
        count(locale, totals.files, "file", "files"),
        count(locale, totals.folders, "folder", "folders")
    )
}

/// What the contents count left out, so a total is never presented as complete
/// when it is not.
fn not_counted(locale: &NumberLocale, totals: &Totals) -> Option<String> {
    let parts: Vec<String> = [
        (totals.links, "link (not followed)", "links (not followed)"),
        (totals.others, "special item", "special items"),
        (totals.unreadable, "unreadable item", "unreadable items"),
        (
            totals.other_filesystems,
            "folder on another filesystem",
            "folders on other filesystems",
        ),
    ]
    .into_iter()
    .filter(|(number, ..)| *number > 0)
    .map(|(number, one, many)| count(locale, number, one, many))
    .collect();
    (!parts.is_empty()).then(|| format!("Not counted: {}", parts.join(", ")))
}

fn several(totals: &Selection, locale: &NumberLocale) -> String {
    let kinds: Vec<String> = [
        (totals.files, "file", "files"),
        (totals.folders, "folder", "folders"),
        (totals.links, "link", "links"),
        (totals.others, "other item", "other items"),
    ]
    .into_iter()
    .filter(|(number, ..)| *number > 0)
    .map(|(number, one, many)| count(locale, number, one, many))
    .collect();
    let mut lines = Vec::new();
    if !kinds.is_empty() {
        lines.push(kinds.join(", "));
    }
    if totals.files > 0 {
        lines.push(format!(
            "Size: {} in {}",
            count(locale, totals.bytes, "byte", "bytes"),
            count(locale, totals.files, "file", "files")
        ));
    }
    if totals.unreadable > 0 {
        lines.push(format!(
            "{} could not be read",
            count(locale, totals.unreadable, "item", "items")
        ));
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(note(totals.folders > 0).into());
    lines.join("\n")
}

fn note(folders: bool) -> &'static str {
    if folders {
        "Read-only metadata. Folder contents are not included."
    } else {
        "Read-only metadata."
    }
}

fn count(locale: &NumberLocale, number: u64, one: &str, many: &str) -> String {
    let unit = if number == 1 { one } else { many };
    format!("{} {unit}", locale.format_u64(number))
}

/// `ls -l` style permission bits with set-user-ID, set-group-ID and sticky bits,
/// followed by the octal mode.
fn permissions(mode: u32) -> String {
    let bit = |mask: u32, letter: char| if mode & mask != 0 { letter } else { '-' };
    let execute =
        |mask: u32, special: u32, letter: char| match (mode & mask != 0, mode & special != 0) {
            (true, true) => letter,
            (false, true) => letter.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        };
    let symbolic: String = [
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        execute(0o100, 0o4000, 's'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        execute(0o010, 0o2000, 's'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        execute(0o001, 0o1000, 't'),
    ]
    .into_iter()
    .collect();
    format!("{symbolic} ({:04o})", mode & 0o7777)
}

/// An explicitly labelled UTC time for a platform without a time format.
pub(super) fn utc_text(time: SystemTime) -> String {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            -i128::from(duration.as_secs()) - i128::from(duration.subsec_nanos() != 0)
        }
    };
    let days = seconds.div_euclid(86_400);
    let second = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        second / 3600,
        second % 3600 / 60,
        second % 60
    )
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's method).
fn civil_from_days(days: i128) -> (i128, i128, i128) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i128::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
#[path = "desktop_properties_tests.rs"]
mod tests;
