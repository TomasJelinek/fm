//! How Compare directories judges one same-named pair (Open Salamander's rules,
//! docs/developer/COMPARE_DIRECTORIES.md): names, size, time with tolerance and
//! daylight saving shifts, attributes, and when contents must be read.
use midnight_core::{CompareOptions, EntryMetadata, FilterMode, PanelFilter};
use midnight_platform_common::filter::CompiledFilter;
use std::{
    ffi::OsStr,
    time::{SystemTime, UNIX_EPOCH},
};
use unicase::UniCase;

/// Windows attributes Salamander shows and compares: read-only, hidden, system,
/// archive, temporary, compressed, offline and encrypted.
const COMPARED_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x800 | 0x1000 | 0x4000;
/// POSIX permission bits, setuid, setgid and sticky included.
const COMPARED_MODE: u32 = 0o7777;
const HOUR: i128 = 3600;

/// A name as pairs are matched: raw, or Unicode-folded when case does not matter.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub(super) enum Name {
    Raw(std::ffi::OsString),
    Folded(String),
}
impl Name {
    pub(super) fn new(name: &OsStr, case_sensitive: bool) -> Self {
        match name.to_str() {
            Some(name) if !case_sensitive => Self::Folded(UniCase::unicode(name).to_folded_case()),
            _ => Self::Raw(name.to_owned()),
        }
    }
}

/// The options of one comparison with its ignore masks compiled.
pub(super) struct Criteria {
    pub(super) options: CompareOptions,
    ignore_files: Option<CompiledFilter>,
    ignore_folders: Option<CompiledFilter>,
}

impl Criteria {
    pub(super) fn new(options: &CompareOptions) -> Result<Self, String> {
        options.validate().map_err(str::to_owned)?;
        let compile = |masks: &Option<String>| -> Result<Option<CompiledFilter>, String> {
            match masks.as_deref().map(str::trim) {
                None | Some("") => Ok(None),
                Some(expression) => CompiledFilter::new(&PanelFilter {
                    enabled: true,
                    mode: FilterMode::Masks,
                    expression: expression.to_owned(),
                    case_sensitive: false,
                })
                .map(Some),
            }
        };
        Ok(Self {
            ignore_files: compile(&options.ignore_files)?,
            ignore_folders: compile(&options.ignore_folders)?,
            options: options.clone(),
        })
    }

    /// Whether an ignore mask leaves this entry out on both sides. Folder masks
    /// apply only where folders are compared at all.
    pub(super) fn ignored(&self, name: &OsStr, directory: bool) -> bool {
        let masks = if directory {
            &self.ignore_folders
        } else {
            &self.ignore_files
        };
        masks.as_ref().is_some_and(|masks| masks.matches(name))
    }

    pub(super) fn name(&self, name: &OsStr) -> Name {
        Name::new(name, self.options.case_sensitive)
    }

    /// Judges a same-named pair of regular files by the enabled metadata criteria.
    pub(super) fn files(&self, entries: [&EntryMetadata; 2]) -> Verdict {
        let options = &self.options;
        let [a, b] = entries;
        let mut verdict = Verdict::default();
        if options.by_size {
            match (a.size, b.size) {
                (Some(a), Some(b)) if a > b => verdict.select[0] = true,
                (Some(a), Some(b)) if b > a => verdict.select[1] = true,
                (Some(_), Some(_)) => (),
                _ => verdict.unknown = true,
            }
        }
        if options.by_modified {
            match (a.modified, b.modified) {
                (Some(a), Some(b)) => match self.times(a, b) {
                    Time::Equal => (),
                    Time::Newer(side) => verdict.select[side] = true,
                    Time::Shift(side) => {
                        verdict.dst_shift = true;
                        if !options.ignore_dst_shifts {
                            verdict.select[side] = true;
                        }
                    }
                },
                _ => verdict.unknown = true,
            }
        }
        if options.by_attributes && attributes_differ(a, b) {
            verdict.select = [true; 2];
        }
        if options.by_content {
            match (a.size, b.size) {
                // Different lengths cannot hold the same bytes, whatever the criteria.
                (Some(a), Some(b)) if a != b => verdict.select = [true; 2],
                // Read unless both files are selected already; empty files are equal.
                (Some(size), Some(_)) => {
                    verdict.read_contents = size > 0 && verdict.select != [true; 2];
                }
                _ => verdict.unknown = true,
            }
        }
        verdict
    }

    /// Salamander's time rule: whole seconds; up to the tolerance apart is equal;
    /// one or two hours apart, give or take the tolerance, is a daylight saving shift.
    fn times(&self, a: SystemTime, b: SystemTime) -> Time {
        let delta = seconds(a) - seconds(b);
        let newer = usize::from(delta < 0);
        let distance = delta.abs();
        let tolerance = i128::from(self.options.time_tolerance_seconds);
        if distance <= tolerance {
            Time::Equal
        } else if (distance - HOUR).abs() <= tolerance || (distance - 2 * HOUR).abs() <= tolerance {
            Time::Shift(newer)
        } else {
            Time::Newer(newer)
        }
    }

    /// With Compare attributes of subfolders: a same-named folder pair differs.
    pub(super) fn folders_differ(&self, entries: [&EntryMetadata; 2]) -> bool {
        self.options.recursive
            && self.options.subfolder_attributes
            && attributes_differ(entries[0], entries[1])
    }
}

enum Time {
    Equal,
    Newer(usize),
    Shift(usize),
}

/// How the enabled metadata criteria judge one same-named file pair.
#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Verdict {
    /// The sides the criteria select: the larger and/or newer file, both for
    /// different attributes or different sizes under Compare contents.
    pub(super) select: [bool; 2],
    /// Requested metadata is unavailable: both files are selected as unknown.
    pub(super) unknown: bool,
    /// Compare contents: the bytes must be read to tell.
    pub(super) read_contents: bool,
    /// The times are one or two hours apart.
    pub(super) dst_shift: bool,
}

impl Verdict {
    /// Anything about the pair differs, or cannot be told without reading.
    pub(super) fn differs(&self) -> bool {
        self.unknown || self.select != [false; 2]
    }
}

/// Compared attributes differ. Unknown attributes on either side compare equal, as
/// Salamander compares attributes only where both panels show them.
fn attributes_differ(a: &EntryMetadata, b: &EntryMetadata) -> bool {
    match (a.native_attributes, b.native_attributes) {
        (Some(a), Some(b)) => return (a ^ b) & COMPARED_ATTRIBUTES != 0,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => (),
    }
    match (a.unix_mode, b.unix_mode) {
        (Some(a), Some(b)) => (a ^ b) & COMPARED_MODE != 0,
        _ => false,
    }
}

/// Whole seconds, rounded down also for dates before the Unix epoch.
fn seconds(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(value) => i128::from(value.as_secs()),
        Err(error) => {
            let value = error.duration();
            -i128::from(value.as_secs()) - i128::from(value.subsec_nanos() != 0)
        }
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
