//! Read-only comparison of the two panels' displayed directories, with the options
//! and semantics of Open Salamander's Compare Directories (see
//! docs/developer/compare-directories.md). Metadata criteria never read file
//! contents; the content criterion reads both files of a pair byte by byte on
//! backend workers. With subfolders, a per-entry outcome (`CompareMarks`) lets the
//! panels show which subfolders and files match.
use crate::{DirectoryIndex, FsError, SelectionRevision, SelectionSnapshot, VisibleDirectory};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[path = "compare_detail.rs"]
mod detail;
pub use detail::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompareId(pub u64);

/// Most seconds the time tolerance accepts.
pub const MAX_COMPARE_TOLERANCE_SECONDS: u32 = 86_400;
/// Most bytes of one ignore-masks expression.
pub const MAX_COMPARE_MASK_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareOptions {
    /// The larger file of a same-named pair is selected.
    pub by_size: bool,
    /// The newer file of a same-named pair is selected (whole seconds, with
    /// `time_tolerance_seconds`).
    pub by_modified: bool,
    /// Both files of a pair whose attributes differ are selected (Windows: read-only,
    /// hidden, system, archive, encrypted, temporary, compressed and offline; elsewhere
    /// the permission bits).
    pub by_attributes: bool,
    /// Same-named files are read byte by byte unless the other criteria already
    /// select both; different bytes, or different sizes, select both files.
    pub by_content: bool,
    /// Same-named folders are compared with everything inside them; a folder that
    /// differs anywhere inside is selected on both sides.
    pub recursive: bool,
    /// With `recursive`: same-named folders whose attributes differ are selected.
    pub subfolder_attributes: bool,
    /// Without `recursive`: folders present on one side only are selected.
    /// (With `recursive` they always are.)
    pub unmatched_directories: bool,
    /// Off: names match without regard to case (Unicode case folding).
    pub case_sensitive: bool,
    /// Modification times this many seconds apart, or fewer, count as equal.
    pub time_tolerance_seconds: u32,
    /// Times exactly one or two hours apart (give or take the tolerance), usually a
    /// daylight saving shift, count as equal.
    pub ignore_dst_shifts: bool,
    /// Files whose names match these masks are left out on both sides.
    pub ignore_files: Option<String>,
    /// Folders whose names match these masks are left out on both sides.
    pub ignore_folders: Option<String>,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            by_size: true,
            by_modified: true,
            by_attributes: false,
            by_content: false,
            recursive: false,
            subfolder_attributes: false,
            unmatched_directories: false,
            // Windows names never differ by case alone.
            case_sensitive: !cfg!(windows),
            time_tolerance_seconds: 2,
            ignore_dst_shifts: false,
            ignore_files: None,
            ignore_folders: None,
        }
    }
}

impl CompareOptions {
    /// The bounds a request must keep; the dialog shows the reason.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.time_tolerance_seconds > MAX_COMPARE_TOLERANCE_SECONDS {
            return Err("The time tolerance must be a whole number from 0 to 86400 seconds.");
        }
        for masks in [&self.ignore_files, &self.ignore_folders]
            .into_iter()
            .flatten()
        {
            if masks.len() > MAX_COMPARE_MASK_BYTES || masks.contains('\0') {
                return Err("Ignore masks may contain at most 1024 bytes and no NUL.");
            }
            if let Err(error) = crate::FilterMasks::parse(masks) {
                return Err(error.message());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CompareSide {
    pub source: VisibleDirectory,
    /// Optional exact displayed tree; comparison includes selectable root rows only.
    pub tree: Option<Arc<crate::TreeProjection>>,
    pub view: Arc<DirectoryIndex>,
    pub view_revision: u64,
    pub selection_revision: SelectionRevision,
}

#[derive(Clone, Debug)]
pub struct CompareRequest {
    pub id: CompareId,
    pub sides: [CompareSide; 2],
    pub options: CompareOptions,
}

/// What a running comparison is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComparePhase {
    /// Listing same-named folders and comparing metadata (contents are read meanwhile).
    #[default]
    Scanning,
    /// Every folder is listed; the remaining file pairs are read byte by byte.
    Contents,
}

/// A running comparison's progress, sampled a few times per second.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareProgress {
    pub phase: ComparePhase,
    /// The folder pair being listed, or the file pair being read, left then right.
    pub current: Option<Arc<[PathBuf; 2]>>,
    /// Displayed folder pairs compared completely, and how many there are.
    pub top_done: u64,
    pub top_total: u64,
    /// Folder pairs listed inside the compared folders.
    pub folders: u64,
    /// File pairs whose metadata was compared inside those folders.
    pub files: u64,
    /// Differences found so far (entries on one side only and differing pairs).
    pub differences: u64,
    /// File pairs read completely so far, and how many need reading so far.
    pub content_files: u64,
    pub content_files_total: u64,
    /// Bytes read from both sides together, and the total both sides need so far.
    pub content_bytes: u64,
    pub content_bytes_total: u64,
}

impl CompareProgress {
    /// The fraction done, when it can be told: displayed folder pairs while folders
    /// are listed, bytes once only contents remain.
    pub fn fraction(&self) -> Option<f32> {
        match self.phase {
            ComparePhase::Scanning if self.top_total > 0 => {
                Some(self.top_done as f32 / self.top_total as f32)
            }
            ComparePhase::Contents if self.content_bytes_total > 0 => {
                Some(self.content_bytes as f32 / self.content_bytes_total as f32)
            }
            _ => None,
        }
    }
}

/// At most this many diagnostics are retained; the counts include the rest.
pub const MAX_COMPARE_DIAGNOSTICS: usize = 8;

/// How one entry compared with its counterpart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompareMark {
    /// Same name on both sides, and nothing the criteria compare differs (for a
    /// folder: nothing inside differs either).
    Same,
    /// Same name on both sides, and something differs (for a folder: something
    /// inside).
    Differs,
    /// Present on this side only.
    OnlyHere,
    /// Could not be compared completely (unreadable, changed while read, a limit).
    Unverified,
}

impl CompareMark {
    fn encode(mark: Option<Self>) -> u8 {
        match mark {
            None => 0,
            Some(Self::Same) => 1,
            Some(Self::Differs) => 2,
            Some(Self::OnlyHere) => 3,
            Some(Self::Unverified) => 4,
        }
    }
    fn decode(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Same),
            2 => Some(Self::Differs),
            3 => Some(Self::OnlyHere),
            4 => Some(Self::Unverified),
            _ => None,
        }
    }
}

/// The name masks used by the worker, retained for rows expanded after comparison.
pub trait CompareIgnoreMasks: std::fmt::Debug + Send + Sync {
    fn ignored(&self, name: &OsStr, directory: bool) -> bool;
}

/// The outcome of each compared entry, including top-level-only comparisons, for both
/// panels. Entries are keyed by their path relative to the compared folder, with
/// names folded when names match without regard to case. Matching files inside a
/// compared folder are not stored: a file that is not stored but whose nearest
/// stored ancestor folder was compared is `Same`, and everything inside a folder on
/// one side only is `OnlyHere`. A stored entry keeps why it got its mark
/// (`CompareDetail`): flags beside the marks, and the few numbers it has in a shared
/// word arena. Bounded by `MAX_BYTES`; when that is reached, only what was stored
/// is shown (`complete` is false), and an entry whose numbers no longer fit keeps
/// its mark and flags only.
#[derive(Clone, Debug)]
pub struct CompareMarks {
    roots: [PathBuf; 2],
    case_sensitive: bool,
    ignore_masks: Option<Arc<dyn CompareIgnoreMasks>>,
    entries: HashMap<OsString, Stored>,
    /// The stored entries' numbers (`CompareDetail::pack`).
    words: Vec<u64>,
    /// The stored keys' and numbers' approximate memory.
    bytes: usize,
    complete: bool,
}

/// One stored entry: each side's mark, the detail's flags and which numbers it has,
/// and where those numbers start in `CompareMarks::words`. Eight bytes: a map entry
/// with its key is as large as one with the two marks alone was (the key's padding).
#[derive(Clone, Copy, Debug)]
struct Stored {
    marks: [u8; 2],
    bits: u16,
    first: u32,
}

impl CompareMarks {
    /// The stored outcome's memory budget.
    pub const MAX_BYTES: usize = 64 * 1024 * 1024;
    /// What one stored entry costs besides its key's bytes and its numbers: the
    /// map's entry (key, marks and detail flags), its control byte and spare room.
    pub const ENTRY_BYTES: usize = 48;

    pub fn new(roots: [PathBuf; 2], case_sensitive: bool) -> Self {
        Self {
            roots,
            case_sensitive,
            ignore_masks: None,
            entries: HashMap::new(),
            words: Vec::new(),
            bytes: 0,
            complete: true,
        }
    }

    /// Whether every compared entry's outcome was stored.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Some compared entries' outcomes could not be stored: entries that are not
    /// stored are not inferred from their folders any more.
    pub fn mark_incomplete(&mut self) {
        self.complete = false;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The memory charged to the budget so far.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn roots(&self) -> &[PathBuf; 2] {
        &self.roots
    }

    /// Reuse the worker's compiled masks without retaining each excluded descendant.
    pub fn set_ignore_masks(&mut self, masks: Arc<dyn CompareIgnoreMasks>) {
        self.ignore_masks = Some(masks);
    }

    /// The key of the child `name` of the entry keyed `parent` (empty: the root).
    pub fn child_key(&self, parent: &OsStr, name: &OsStr) -> OsString {
        let mut key = parent.to_owned();
        if !key.is_empty() {
            key.push("/");
        }
        push_name(&mut key, name, self.case_sensitive);
        key
    }

    /// The key of `relative`, a path below a compared root.
    pub fn key(&self, relative: &Path) -> Option<OsString> {
        let mut key = OsString::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return None;
            };
            if !key.is_empty() {
                key.push("/");
            }
            push_name(&mut key, name, self.case_sensitive);
        }
        (!key.is_empty()).then_some(key)
    }

    /// Stores `mark` for `side` of `key`, without a detail (`record`).
    pub fn set(&mut self, key: &OsStr, side: usize, mark: CompareMark) -> bool {
        self.record(key, Some(side), mark, &CompareDetail::default())
    }

    /// Stores `mark` for `side` of `key` (both sides when None) and merges `detail`
    /// into what the entry already tells. A worse outcome is never replaced by a
    /// better one (Differs, OnlyHere and Unverified stay). False once the budget is
    /// spent: nothing more is stored and the marks are incomplete. Numbers that no
    /// longer fit are left out; the mark and flags are still stored.
    pub fn record(
        &mut self,
        key: &OsStr,
        side: Option<usize>,
        mark: CompareMark,
        detail: &CompareDetail,
    ) -> bool {
        let sides = side.map_or(0..2, |side| side..side + 1);
        if let Some(mut stored) = self.entries.get(key).copied() {
            for slot in sides {
                let slot = &mut stored.marks[slot];
                if CompareMark::decode(*slot).is_none_or(|old| old == CompareMark::Same) {
                    *slot = CompareMark::encode(Some(mark));
                }
            }
            if !detail.is_empty() {
                let merged = self.detail_of(&stored).merged(detail);
                self.store_detail(&mut stored, &merged);
            }
            if let Some(entry) = self.entries.get_mut(key) {
                *entry = stored;
            }
            return true;
        }
        let cost = key.len() + Self::ENTRY_BYTES;
        if self.bytes + cost > Self::MAX_BYTES {
            self.complete = false;
            return false;
        }
        self.bytes += cost;
        let mut stored = Stored {
            marks: [0; 2],
            bits: 0,
            first: 0,
        };
        for slot in sides {
            stored.marks[slot] = CompareMark::encode(Some(mark));
        }
        if !detail.is_empty() {
            self.store_detail(&mut stored, detail);
        }
        self.entries.insert(key.to_owned(), stored);
        true
    }

    /// Packs `detail` for `stored`: in place when it needs as many words as before,
    /// otherwise at the end of the arena (the replaced words stay charged).
    fn store_detail(&mut self, stored: &mut Stored, detail: &CompareDetail) {
        let mut words = Vec::new();
        let bits = detail.pack(&mut words);
        let old = CompareDetail::words(stored.bits);
        if words.len() == old {
            let first = stored.first as usize;
            self.words[first..first + old].copy_from_slice(&words);
            stored.bits = bits;
            return;
        }
        let cost = words.len() * size_of::<u64>();
        match u32::try_from(self.words.len()) {
            Ok(first) if self.bytes + cost <= Self::MAX_BYTES => {
                self.bytes += cost;
                self.words.extend(words);
                stored.bits = bits;
                stored.first = first;
            }
            // The flags still fit; the numbers stored before, if any, stay.
            _ => stored.bits = CompareDetail::with_numbers_of(bits, stored.bits),
        }
    }

    fn detail_of(&self, stored: &Stored) -> CompareDetail {
        let first = stored.first as usize;
        let count = CompareDetail::words(stored.bits);
        CompareDetail::unpack(
            stored.bits,
            self.words.get(first..first + count).unwrap_or_default(),
        )
    }

    /// The outcome stored for `side` of `key`, without inference.
    pub fn stored(&self, key: &OsStr, side: usize) -> Option<CompareMark> {
        self.entries
            .get(key)
            .and_then(|stored| CompareMark::decode(stored.marks[side]))
    }

    /// Why the entry `key` got its stored marks; empty when it has no detail.
    pub fn detail(&self, key: &OsStr) -> CompareDetail {
        self.entries
            .get(key)
            .map(|stored| self.detail_of(stored))
            .unwrap_or_default()
    }

    /// The outcome of the displayed entry `path` in panel `side`, or None when it
    /// was not compared (or the marks are incomplete and it was not stored).
    pub fn mark(&self, side: usize, path: &Path) -> Option<CompareMark> {
        self.outcome(side, path).map(|(mark, _)| mark)
    }

    /// `mark` with why: the stored detail of `path`, or an empty one when its mark
    /// is inferred from a folder above it. If ignore masks could exclude a file
    /// or folder of this name, only a stored verdict is returned. Call
    /// [`Self::entry_outcome`] when the row's kind is available.
    pub fn outcome(&self, side: usize, path: &Path) -> Option<(CompareMark, CompareDetail)> {
        self.lookup(side, path, None)
    }

    /// Looks up a displayed row with its known kind. An inferred mark requires its
    /// name and all parent folders to pass the same masks as the comparison.
    pub fn entry_outcome(
        &self,
        side: usize,
        path: &Path,
        directory: bool,
    ) -> Option<(CompareMark, CompareDetail)> {
        self.lookup(side, path, Some(directory))
    }

    fn lookup(
        &self,
        side: usize,
        path: &Path,
        directory: Option<bool>,
    ) -> Option<(CompareMark, CompareDetail)> {
        let relative = path.strip_prefix(self.roots.get(side)?).ok()?;
        let mut components = relative.components().peekable();
        let mut key = OsString::new();
        let mut may_infer = true;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return None;
            };
            if let Some(masks) = &self.ignore_masks {
                let kind = if components.peek().is_some() {
                    Some(true)
                } else {
                    directory
                };
                match kind {
                    Some(folder) if masks.ignored(name, folder) => return None,
                    // A path alone does not describe its last component. Its stored
                    // verdict is authoritative; infer only if both kinds are included.
                    None => may_infer = !masks.ignored(name, true) && !masks.ignored(name, false),
                    _ => (),
                }
            }
            if !key.is_empty() {
                key.push("/");
            }
            push_name(&mut key, name, self.case_sensitive);
        }
        if key.is_empty() {
            return None;
        }
        if let Some(stored) = self.entries.get(key.as_os_str())
            && let Some(mark) = CompareMark::decode(stored.marks[side])
        {
            return Some((mark, self.detail_of(stored)));
        }
        if !may_infer {
            return None;
        }
        // The nearest stored ancestor tells about entries that were not stored.
        for parent in Path::new(&key).ancestors().skip(1) {
            let ancestor = parent.as_os_str();
            match self.stored(ancestor, side) {
                Some(CompareMark::OnlyHere) => {
                    return Some((CompareMark::OnlyHere, CompareDetail::default()));
                }
                Some(CompareMark::Same) if self.complete => {
                    return Some((CompareMark::Same, CompareDetail::default()));
                }
                // An attribute or name difference can stop a folder before its
                // children are examined. Only a walked folder has inside counts.
                Some(CompareMark::Differs)
                    if self.complete && self.detail(ancestor).inside.is_some() =>
                {
                    return Some((CompareMark::Same, CompareDetail::default()));
                }
                Some(_) => return None,
                None => continue,
            }
        }
        None
    }
}

fn push_name(key: &mut OsString, name: &OsStr, case_sensitive: bool) {
    match name.to_str() {
        Some(name) if !case_sensitive => key.push(unicase::UniCase::unicode(name).to_folded_case()),
        _ => key.push(name),
    }
}

#[derive(Clone, Debug)]
pub struct CompareResult {
    pub request: Arc<CompareRequest>,
    pub selections: [Arc<SelectionSnapshot>; 2],
    /// Worker-built occurrence selections; these never authorize native operations.
    pub tree_replacements: [Option<Arc<crate::TreeSelectionReplacement>>; 2],
    /// Each compared entry's outcome, for the panels' rows.
    pub marks: Option<Arc<CompareMarks>>,
    /// Pairs/items whose requested metadata is unavailable; these are selected.
    pub unknown: usize,
    /// Case-insensitive name groups that cannot be paired uniquely.
    pub ambiguous: usize,
    /// Same-named displayed folder pairs that differ (Include subfolders).
    pub differing_folders: usize,
    /// Same-named displayed folder pairs compared (Include subfolders).
    pub compared_folders: usize,
    /// File pairs whose bytes differ (Compare contents).
    pub content_differences: usize,
    /// File pairs read completely, and the bytes read from both sides together.
    pub content_files: u64,
    pub content_bytes: u64,
    /// Folder pairs listed and file pairs compared inside the compared folders.
    pub walked_folders: u64,
    pub walked_files: u64,
    /// File pairs whose times are one or two hours apart (a daylight saving shift):
    /// counted as different, or as equal with `ignore_dst_shifts`.
    pub dst_shifts: u64,
    /// Links, special files and mount boundaries present on both sides; they are
    /// matched by name only and never followed.
    pub skipped: u64,
    /// Entries or folders that could not be read; their folders count as different.
    pub errors: u64,
    /// A resource limit ended part of the walk; unfinished folders count as different.
    pub limited: bool,
    /// At most MAX_COMPARE_DIAGNOSTICS samples of the errors above.
    pub diagnostics: Vec<FsError>,
}

impl CompareResult {
    /// Nothing differs and nothing was left unverified: Salamander's "The files in
    /// two directories are identical" (with contents) or "appear to be identical".
    pub fn identical(&self) -> bool {
        self.selections.iter().all(|selection| selection.is_empty())
            && self.unknown == 0
            && self.ambiguous == 0
            && self.errors == 0
            && !self.limited
    }
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
