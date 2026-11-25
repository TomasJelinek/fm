//! Short names for the drive toolbar. The drive bar and the drive menu show each mount
//! by a name instead of its whole path: `/` for the root, the drive's label when it has
//! one (a filesystem's udev label; in the native filesystem package also a GVfs
//! location's name), else the mount's own folder. A name longer than 16 columns keeps
//! its start and end around `…`. Drives
//! whose names are equal take folders from the end of their paths until every name
//! differs (`alice/USB`, `mnt/USB`). The path itself is never changed: the toolbar's
//! tooltips and accessible descriptions spell it out, and navigation uses it.
//!
//! The application's own Linux adapter (`crates/midnight-platform-linux`) applies the
//! same rules in its own copy of this module: the package depends on no host crate.
use std::{collections::HashMap, fmt::Write as _};

#[cfg(test)]
#[path = "drive_names_tests.rs"]
mod tests;

/// Columns a shortened name takes at most, its ellipsis included: room for an ext4 label
/// (16 bytes) or a FAT or exFAT label whole. East Asian wide characters take two.
const MAX_COLUMNS: usize = 16;
/// Columns kept before the ellipsis; the rest of the room, less the ellipsis, after it.
const HEAD_COLUMNS: usize = 8;

/// One listed mount: its raw path and its filesystem label, if it has one.
pub(crate) struct Mount<'a> {
    pub(crate) path: &'a [u8],
    pub(crate) label: Option<&'a [u8]>,
}

/// The name the drive toolbar shows for each of `mounts`, in the same order. Names of
/// distinct paths differ.
pub(crate) fn names(mounts: &[Mount<'_>]) -> Vec<String> {
    let parts: Vec<Vec<&[u8]>> = mounts
        .iter()
        .map(|mount| {
            mount
                .path
                .split(|byte| *byte == b'/')
                .filter(|part| !part.is_empty())
                .collect()
        })
        .collect();
    let mut steps = vec![0; mounts.len()];
    let mut names: Vec<String> = mounts
        .iter()
        .zip(&parts)
        .map(|(mount, parts)| candidate(mount, parts, 0).unwrap_or_default())
        .collect();
    loop {
        // Every drive of a shared name takes one step towards its whole path. Steps are
        // finite and the whole paths of distinct mounts differ, so this ends.
        let mut sharing: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, name) in names.iter().enumerate() {
            sharing.entry(name).or_default().push(index);
        }
        let crowded: Vec<usize> = sharing
            .into_values()
            .filter(|indices| indices.len() > 1)
            .flatten()
            .collect();
        let mut moved = false;
        for index in crowded {
            if let Some(next) = candidate(&mounts[index], &parts[index], steps[index] + 1) {
                steps[index] += 1;
                names[index] = next;
                moved = true;
            }
        }
        if !moved {
            return names;
        }
    }
}

/// The name of `mount` at `step`: its own name first, then its last 1, 2, … path
/// folders shortened, finally all of them whole; `None` past that.
fn candidate(mount: &Mount<'_>, parts: &[&[u8]], step: usize) -> Option<String> {
    let Some(last) = parts.last() else {
        return (step == 0).then(|| "/".to_owned());
    };
    if step == 0 {
        let label = mount
            .label
            .map(|label| text(label).trim().to_owned())
            .filter(|label| !label.is_empty());
        return Some(shorten(&label.unwrap_or_else(|| text(last))));
    }
    let count = step.min(parts.len());
    let tail = parts[parts.len() - count..]
        .iter()
        .map(|part| text(part))
        .collect::<Vec<_>>()
        .join("/");
    match step {
        step if step <= parts.len() => Some(shorten(&tail)),
        step if step == parts.len() + 1 => Some(tail),
        _ => None,
    }
}

/// Native bytes as text for one line: control characters and bytes that are not UTF-8
/// show escaped (`\t`, `\xFF`), so distinct raw names stay distinct.
fn text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for character in chunk.valid().chars() {
            if character.is_control() {
                text.extend(character.escape_default());
            } else {
                text.push(character);
            }
        }
        for byte in chunk.invalid() {
            // Writing to a String cannot fail.
            let _ = write!(text, "\\x{byte:02X}");
        }
    }
    text
}

/// `name` if it fits [`MAX_COLUMNS`], else its start and end around `…`, never splitting
/// a letter from its marks or an emoji sequence.
fn shorten(name: &str) -> String {
    let clusters = clusters(name);
    if clusters.iter().map(|(_, columns)| columns).sum::<usize>() <= MAX_COLUMNS {
        return name.to_owned();
    }
    let mut head = String::new();
    let mut used = 0;
    for (cluster, columns) in &clusters {
        if used + columns > HEAD_COLUMNS {
            break;
        }
        head.push_str(cluster);
        used += columns;
    }
    let mut tail = Vec::new();
    used = 0;
    for (cluster, columns) in clusters.iter().rev() {
        if used + columns > MAX_COLUMNS - 1 - HEAD_COLUMNS {
            break;
        }
        tail.push(*cluster);
        used += columns;
    }
    tail.reverse();
    format!("{}…{}", head.trim_end(), tail.concat().trim_start())
}

/// `text` in user-perceived characters with the columns each takes: a character with the
/// marks, variation selectors and emoji modifiers that follow it, and what a zero width
/// joiner joins to it. An approximation of Unicode's grapheme clusters, without tables.
fn clusters(text: &str) -> Vec<(&str, usize)> {
    let mut clusters = Vec::new();
    let mut start = 0;
    let mut columns = 0;
    let mut joined = false;
    for (index, character) in text.char_indices() {
        if index > 0 && !joined && !extends(character) {
            clusters.push((&text[start..index], columns));
            start = index;
            columns = 0;
        }
        if index == start {
            columns = width(character);
        }
        joined = character == '\u{200d}';
    }
    if !text.is_empty() {
        clusters.push((&text[start..], columns));
    }
    clusters
}

/// Characters that belong to the one before them. Code points, not character literals:
/// none of these is shown as text by itself.
fn extends(character: char) -> bool {
    matches!(u32::from(character),
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x200d
            | 0x20d0..=0x20ff
            | 0xfe00..=0xfe0f
            | 0xfe20..=0xfe2f
            | 0x1f3fb..=0x1f3ff
            | 0xe0020..=0xe007f
            | 0xe0100..=0xe01ef)
}

/// Columns of a character starting a cluster: two for East Asian wide characters and
/// emoji pictographs, one otherwise.
fn width(character: char) -> usize {
    if extends(character) {
        return 0;
    }
    match u32::from(character) {
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x1f300..=0x1f64f
        | 0x1f900..=0x1f9ff
        | 0x20000..=0x2fffd
        | 0x30000..=0x3fffd => 2,
        _ => 1,
    }
}
