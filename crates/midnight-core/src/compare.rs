//! Read-only comparison of the two panels' displayed directories, with the options
//! and semantics of Open Salamander's Compare Directories (see
//! docs/developer/COMPARE_DIRECTORIES.md). Metadata criteria never read file
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

/// The outcome of each compared entry of an Include subfolders comparison, for both
/// panels. Entries are keyed by their path relative to the compared folder, with
/// names folded when names match without regard to case. Matching files inside a
/// compared folder are not stored: a file that is not stored but whose nearest
/// stored ancestor folder was compared is `Same`, and everything inside a folder on
/// one side only is `OnlyHere`. Bounded by `MAX_BYTES`; when that is reached, only
/// what was stored is shown (`complete` is false).
#[derive(Clone, Debug)]
pub struct CompareMarks {
    roots: [PathBuf; 2],
    case_sensitive: bool,
    entries: HashMap<OsString, [u8; 2]>,
    /// The stored keys' approximate memory.
    bytes: usize,
    complete: bool,
}

impl CompareMarks {
    /// The stored outcome's memory budget.
    pub const MAX_BYTES: usize = 64 * 1024 * 1024;

    pub fn new(roots: [PathBuf; 2], case_sensitive: bool) -> Self {
        Self {
            roots,
            case_sensitive,
            entries: HashMap::new(),
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

    pub fn roots(&self) -> &[PathBuf; 2] {
        &self.roots
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

    /// Stores `mark` for `side` of `key`. A worse outcome is never replaced by a
    /// better one (Differs, OnlyHere and Unverified stay). False once the budget is
    /// spent: nothing more is stored and the marks are incomplete.
    pub fn set(&mut self, key: &OsStr, side: usize, mark: CompareMark) -> bool {
        if let Some(existing) = self.entries.get_mut(key) {
            let slot = &mut existing[side];
            if CompareMark::decode(*slot).is_none_or(|old| old == CompareMark::Same) {
                *slot = CompareMark::encode(Some(mark));
            }
            return true;
        }
        let cost = key.len() + 48;
        if self.bytes + cost > Self::MAX_BYTES {
            self.complete = false;
            return false;
        }
        self.bytes += cost;
        let mut value = [0; 2];
        value[side] = CompareMark::encode(Some(mark));
        self.entries.insert(key.to_owned(), value);
        true
    }

    /// The outcome stored for `side` of `key`, without inference.
    pub fn stored(&self, key: &OsStr, side: usize) -> Option<CompareMark> {
        self.entries
            .get(key)
            .and_then(|marks| CompareMark::decode(marks[side]))
    }

    /// The outcome of the displayed entry `path` in panel `side`, or None when it
    /// was not compared (or the marks are incomplete and it was not stored).
    pub fn mark(&self, side: usize, path: &Path) -> Option<CompareMark> {
        let relative = path.strip_prefix(&self.roots[side]).ok()?;
        // The keys of the entry and of each of its ancestors, the entry's last.
        let mut prefixes = Vec::new();
        let mut key = OsString::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return None;
            };
            if !key.is_empty() {
                key.push("/");
            }
            push_name(&mut key, name, self.case_sensitive);
            prefixes.push(key.clone());
        }
        let own = prefixes.pop()?;
        if let Some(mark) = self.stored(&own, side) {
            return Some(mark);
        }
        // The nearest stored ancestor tells about entries that were not stored.
        for ancestor in prefixes.iter().rev() {
            match self.stored(ancestor, side) {
                Some(CompareMark::OnlyHere) => return Some(CompareMark::OnlyHere),
                Some(CompareMark::Same | CompareMark::Differs) if self.complete => {
                    return Some(CompareMark::Same);
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
    /// Include subfolders: each compared entry's outcome, for the panels' rows.
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
