//! Incremental literal Find. Retains one hit, four navigation candidates and one line's folds.
use super::{TextComparison, visible_controls};
use crate::{FindQuery, FindState, check_cancel};
use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};
use unicase::UniCase;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
#[path = "text_find_tests.rs"]
mod tests;

const TURN_BYTES: usize = 256 * 1024;
const TURN_ROWS: usize = 64;
const MAX_QUERY_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub row: usize,
    pub side: usize,
    pub line: usize,
    pub range: Range<usize>,
}

impl Hit {
    fn key(&self) -> (usize, usize, usize) {
        (self.row, self.side, self.range.start)
    }
}

type Candidate = Option<(Hit, usize)>;

struct Scan {
    row: usize,
    backwards: bool,
    anchor: (usize, usize, usize),
    advance: bool,
    count: usize,
    first: Candidate,
    last: Candidate,
    previous: Candidate,
    next: Candidate,
    folded: FoldedLine,
    pattern: String,
    failure: Vec<usize>,
    target: Option<usize>,
    delta: i64,
}

#[derive(Default)]
pub(super) struct TextFind {
    query: FindQuery,
    pub state: Arc<FindState>,
    pub selected: Option<Hit>,
    pub reveal: bool,
    pub horizontal_bounds: Option<[f64; 2]>,
    scan: Option<Scan>,
    /// One count checkpoint per 64 aligned rows, rather than one allocation per match.
    checkpoints: Vec<(usize, usize)>,
}

impl TextFind {
    pub fn pending(&self) -> bool {
        self.scan.is_some()
    }
}

impl TextComparison {
    pub fn begin_find(&mut self, query: FindQuery, backwards: bool, revision: u64) {
        let previous = (query == self.find.query)
            .then(|| self.find.selected.take())
            .flatten();
        self.find = TextFind::default();
        self.find.query = query;
        Arc::make_mut(&mut self.find.state).revision = revision;
        if self.find.query.text.is_empty() {
            return;
        }
        if self.find.query.text.len() > MAX_QUERY_BYTES {
            Arc::make_mut(&mut self.find.state).message =
                "Find supports up to 4096 UTF-8 bytes.".into();
            return;
        }
        let state = Arc::make_mut(&mut self.find.state);
        state.searching = true;
        state.message = "Searching…".into();
        self.find.checkpoints.push((0, 0));
        let pattern = folded_pattern(&self.find.query);
        let failure = literal_prefix(pattern.as_bytes());
        self.find.scan = Some(Scan {
            row: 0,
            backwards,
            anchor: previous.as_ref().map_or((self.top, 0, 0), Hit::key),
            advance: previous.is_some(),
            count: 0,
            first: None,
            last: None,
            previous: None,
            next: None,
            folded: FoldedLine::default(),
            pattern,
            failure,
            target: None,
            delta: 0,
        });
    }

    pub fn advance_find(&mut self, delta: i64, revision: u64) {
        Arc::make_mut(&mut self.find.state).revision = revision;
        if let Some(scan) = self.find.scan.as_mut()
            && scan.target.is_none()
        {
            scan.delta = scan.delta.saturating_add(delta);
            return;
        }
        if self.find.state.matches == 0 {
            return;
        }
        let base = self
            .find
            .scan
            .as_ref()
            .and_then(|scan| scan.target)
            .unwrap_or(self.find.state.active);
        let ordinal = shifted(base, delta, self.find.state.matches);
        self.start_find_target(ordinal);
    }

    fn start_find_target(&mut self, ordinal: usize) {
        let checkpoint = self
            .find
            .checkpoints
            .partition_point(|(_, count)| *count < ordinal)
            .saturating_sub(1);
        let (row, count) = self
            .find
            .checkpoints
            .get(checkpoint)
            .copied()
            .unwrap_or((0, 0));
        let pattern = folded_pattern(&self.find.query);
        let failure = literal_prefix(pattern.as_bytes());
        self.find.scan = Some(Scan {
            row,
            count,
            target: Some(ordinal),
            delta: 0,
            pattern,
            failure,
            backwards: false,
            anchor: (0, 0, 0),
            advance: false,
            first: None,
            last: None,
            previous: None,
            next: None,
            folded: FoldedLine::default(),
        });
        let state = Arc::make_mut(&mut self.find.state);
        state.searching = true;
        state.message = "Searching…".into();
    }

    /// One CPU turn, then the worker consumes pending input/close/resize. A turn ends
    /// after 64 rows or 256 KiB, with at most one aligned row of overshoot. Admitted
    /// source lines are at most 16 KiB each; visible control escapes can enlarge them.
    /// True means a final result needs painting; partial results never move the viewport.
    pub fn find_turn(&mut self, cancel: &AtomicBool) -> Result<bool, String> {
        let Some(scan) = self.find.scan.as_mut() else {
            return Ok(false);
        };
        let mut bytes = 0;
        let mut target_found = false;
        let last = self.rows.len().min(scan.row + TURN_ROWS);
        'rows: while scan.row < last && bytes < TURN_BYTES {
            check_cancel(cancel)?;
            let row = &self.rows[scan.row];
            for (side, index) in [row.left(), row.right()].into_iter().enumerate() {
                let Some(line) = index else { continue };
                let text =
                    visible_controls(self.lines[side].line(line).trim_end_matches(['\r', '\n']));
                bytes += text.len();
                let haystack = if self.find.query.case_sensitive {
                    text.as_ref()
                } else {
                    scan.folded.set(&text, cancel)?;
                    &scan.folded.text
                };
                for (matched, folded_range) in
                    LiteralMatches::new(haystack, &scan.pattern, &scan.failure).enumerate()
                {
                    if matched.is_multiple_of(1024) {
                        check_cancel(cancel)?;
                    }
                    let range = if self.find.query.case_sensitive {
                        Some(folded_range)
                    } else {
                        scan.folded.original(folded_range)
                    };
                    let Some(range) = range else { continue };
                    if self.find.query.whole_words && !whole_word(&text, &range) {
                        continue;
                    }
                    let hit = Hit {
                        row: scan.row,
                        side,
                        line,
                        range,
                    };
                    scan.count += 1;
                    let candidate = (hit, scan.count);
                    if let Some(target) = scan.target {
                        if scan.count == target {
                            scan.last = Some(candidate);
                            target_found = true;
                            break 'rows;
                        }
                        continue;
                    }
                    if scan.first.is_none() {
                        scan.first = Some(candidate.clone());
                    }
                    let key = candidate.0.key();
                    if key < scan.anchor {
                        scan.previous = Some(candidate.clone());
                    }
                    if scan.next.is_none()
                        && (key > scan.anchor || (!scan.advance && key == scan.anchor))
                    {
                        scan.next = Some(candidate.clone());
                    }
                    scan.last = Some(candidate);
                }
            }
            scan.row += 1;
            if scan.target.is_none() && scan.row.is_multiple_of(TURN_ROWS) {
                self.find.checkpoints.push((scan.row, scan.count));
            }
        }
        if !target_found && scan.row < self.rows.len() {
            return Ok(false);
        }
        let Some(scan) = self.find.scan.take() else {
            return Ok(false);
        };
        let selected = if scan.target.is_some() {
            scan.last
        } else if scan.backwards {
            scan.previous.or(scan.last)
        } else {
            scan.next.or(scan.first)
        };
        if scan.target.is_none() {
            Arc::make_mut(&mut self.find.state).matches = scan.count;
        }
        if let Some((hit, ordinal)) = selected {
            if scan.target.is_none() && scan.delta != 0 {
                let wanted = shifted(ordinal, scan.delta, self.find.state.matches);
                if wanted != ordinal {
                    self.start_find_target(wanted);
                    return Ok(false);
                }
            }
            let state = Arc::make_mut(&mut self.find.state);
            state.searching = false;
            state.active = ordinal;
            state.side = hit.side;
            state.line = hit.line + 1;
            state.message = format!(
                "Match {ordinal} of {} · File {} · line {}",
                state.matches,
                if hit.side == 0 { "A" } else { "B" },
                hit.line + 1,
            );
            self.find.selected = Some(hit);
            self.find.reveal = true;
        } else {
            let state = Arc::make_mut(&mut self.find.state);
            state.searching = false;
            state.message = "No matches".into();
        }
        Ok(true)
    }
}

fn folded_pattern(query: &FindQuery) -> String {
    if query.case_sensitive {
        query.text.clone()
    } else {
        UniCase::unicode(&query.text).to_folded_case()
    }
}

fn shifted(ordinal: usize, delta: i64, total: usize) -> usize {
    ((ordinal.saturating_sub(1) as i128 + i128::from(delta)).rem_euclid(total as i128) + 1) as usize
}

fn literal_prefix(pattern: &[u8]) -> Vec<usize> {
    let mut failure = vec![0; pattern.len()];
    let mut matched = 0;
    for index in 1..pattern.len() {
        while matched > 0 && pattern[index] != pattern[matched] {
            matched = failure[matched - 1];
        }
        if pattern[index] == pattern[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }
    failure
}

/// Linear literal matching includes overlaps. Rejecting a partial folded scalar must
/// not consume the following valid whole-scalar candidate (for example SS in sß).
struct LiteralMatches<'a> {
    text: &'a [u8],
    pattern: &'a [u8],
    failure: &'a [usize],
    cursor: usize,
    matched: usize,
}

impl<'a> LiteralMatches<'a> {
    fn new(text: &'a str, pattern: &'a str, failure: &'a [usize]) -> Self {
        Self {
            text: text.as_bytes(),
            pattern: pattern.as_bytes(),
            failure,
            cursor: 0,
            matched: 0,
        }
    }
}

impl Iterator for LiteralMatches<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pattern.is_empty() {
            return None;
        }
        while let Some(&value) = self.text.get(self.cursor) {
            while self.matched > 0 && self.pattern[self.matched] != value {
                self.matched = self.failure[self.matched - 1];
            }
            if self.pattern[self.matched] == value {
                self.matched += 1;
            }
            self.cursor += 1;
            if self.matched == self.pattern.len() {
                let range = self.cursor - self.matched..self.cursor;
                self.matched = self.failure[self.matched - 1];
                return Some(range);
            }
        }
        None
    }
}

fn whole_word(text: &str, range: &Range<usize>) -> bool {
    !text[..range.start]
        .chars()
        .next_back()
        .is_some_and(word_character)
        && !text[range.end..].chars().next().is_some_and(word_character)
}

// Same Unicode semantics as the filesystem Find engine, including marks/connectors/formats.
fn word_character(value: char) -> bool {
    if value.is_ascii() {
        return value.is_ascii_alphanumeric() || value == '_';
    }
    if value.is_alphanumeric() {
        return true;
    }
    let mut bytes = [0; 5];
    bytes[0] = b'a';
    let length = 1 + value.encode_utf8(&mut bytes[1..]).len();
    let Ok(text) = std::str::from_utf8(&bytes[..length]) else {
        return false;
    };
    text.unicode_words()
        .next()
        .is_some_and(|word| word.len() == text.len())
}

#[derive(Default)]
struct FoldedLine {
    text: String,
    /// Only whole original scalar boundaries are valid. Expansion interiors stay absent.
    boundaries: Vec<Option<usize>>,
    ascii: bool,
    /// Bounded scalar cache avoids allocating for every repeated non-ASCII character.
    scalars: HashMap<char, String>,
}

impl FoldedLine {
    fn set(&mut self, source: &str, cancel: &AtomicBool) -> Result<(), String> {
        check_cancel(cancel)?;
        self.text.clear();
        self.boundaries.clear();
        self.ascii = source.is_ascii();
        if self.ascii {
            self.text.push_str(source);
            self.text.make_ascii_lowercase();
            return Ok(());
        }
        self.boundaries.push(Some(0));
        for (unit, (offset, value)) in source.char_indices().enumerate() {
            if unit.is_multiple_of(1024) {
                check_cancel(cancel)?;
            }
            let start = self.text.len();
            if value.is_ascii() {
                self.text.push(value.to_ascii_lowercase());
            } else {
                if self.scalars.len() >= 512 && !self.scalars.contains_key(&value) {
                    self.scalars.clear();
                }
                let folded = self.scalars.entry(value).or_insert_with(|| {
                    let mut bytes = [0; 4];
                    UniCase::unicode(value.encode_utf8(&mut bytes)).to_folded_case()
                });
                self.text.push_str(folded);
            }
            self.boundaries.resize(self.text.len() + 1, None);
            self.boundaries[start] = Some(offset);
            self.boundaries[self.text.len()] = Some(offset + value.len_utf8());
        }
        Ok(())
    }

    fn original(&self, range: Range<usize>) -> Option<Range<usize>> {
        if self.ascii {
            return Some(range);
        }
        Some((*self.boundaries.get(range.start)?)?..(*self.boundaries.get(range.end)?)?)
    }
}
